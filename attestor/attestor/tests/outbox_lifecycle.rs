//! Regression tests for historical Outbox authorization using the production RPC paths.
use std::sync::{Arc, Mutex};

use alloy::primitives::{address, Address, B256};
use alloy::providers::ProviderBuilder;
use alloy::rpc::types::Log;
use alloy::sol_types::{SolCall, SolEvent, SolValue};
use attestor::tasks::write_ability::{listener, reobservation, resolver};
use axum::{extract::State, routing::post, Json, Router};
use serde_json::{json, Value};
use write_ability::abi::{IChainInfo, IOutbox, IOutboxDiscovery};
use write_ability::envelope::ReobservationRequest;

const A: Address = address!("00000000000000000000000000000000000000aa");
const B: Address = address!("00000000000000000000000000000000000000bb");
const ROGUE: Address = address!("00000000000000000000000000000000000000cc");
const REGISTRY: Address = address!("0000000000000000000000000000000000000011");
const REPLACEMENT: Address = address!("0000000000000000000000000000000000000022");

fn route() -> resolver::ResolvedRoute {
    resolver::ResolvedRoute {
        chain_key: 7,
        destination_chain_key: B256::repeat_byte(7),
        creditcoin_chain_id: 42,
    }
}

fn message(outbox: Address, block: u64, id: u8) -> Log {
    let event = IOutbox::MessagePublished {
        messageId: B256::repeat_byte(id),
        emitterAddress: B256::repeat_byte(4),
        sequence: 1,
        canAck: false,
        payload: vec![id].into(),
    };
    Log {
        inner: alloy::primitives::Log {
            address: outbox,
            data: event.encode_log_data(),
        },
        block_number: Some(block),
        block_hash: Some(B256::repeat_byte(block as u8)),
        transaction_hash: Some(B256::repeat_byte(id)),
        transaction_index: Some(0),
        log_index: Some(0),
        ..Default::default()
    }
}

struct Scenario {
    logs: Vec<Log>,
    fail_history: bool,
    fail_history_at: Option<u64>,
    log_cap: Option<usize>,
    ignore_log_filter: bool,
    calls: Vec<Value>,
}
struct Rpc {
    state: Arc<Mutex<Scenario>>,
    url: url::Url,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Rpc {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Rpc {
    async fn new(logs: Vec<Log>) -> Self {
        let state = Arc::new(Mutex::new(Scenario {
            logs,
            fail_history: false,
            fail_history_at: None,
            log_cap: None,
            ignore_log_filter: false,
            calls: vec![],
        }));
        let app = Router::new()
            .route("/", post(handle))
            .with_state(state.clone());
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", socket.local_addr().unwrap())
            .parse()
            .unwrap();
        let task = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
        Self { state, url, task }
    }
}
fn height(value: &Value) -> u64 {
    u64::from_str_radix(
        value
            .as_str()
            .expect("historical block must be explicit")
            .strip_prefix("0x")
            .unwrap(),
        16,
    )
    .unwrap()
}
async fn handle(
    State(state): State<Arc<Mutex<Scenario>>>,
    Json(request): Json<Value>,
) -> Json<Value> {
    let mut state = state.lock().unwrap();
    state.calls.push(request.clone());
    let p = &request["params"];
    let result = match request["method"].as_str().unwrap() {
        "eth_chainId" => json!("0x2a"),
        "eth_blockNumber" => json!("0x8c"),
        "eth_getBlockByNumber" => {
            let number = if p[0] == "finalized" {
                140
            } else {
                height(&p[0])
            };
            let mut block = alloy::rpc::types::Block::<B256>::default();
            block.header.inner.number = number;
            block.header.hash = B256::repeat_byte(number as u8);
            block.transactions = alloy::network::primitives::BlockTransactions::Hashes(
                state
                    .logs
                    .iter()
                    .filter(|l| l.block_number == Some(number))
                    .map(|l| l.transaction_hash.unwrap())
                    .collect(),
            );
            json!(block)
        }
        "eth_getTransactionReceipt" => {
            let hash: B256 = serde_json::from_value(p[0].clone()).unwrap();
            let logs: Vec<_> = state
                .logs
                .iter()
                .filter(|l| l.transaction_hash == Some(hash))
                .collect();
            assert!(!logs.is_empty());
            json!({"transactionHash":hash,"transactionIndex":"0x0", "blockHash":logs[0].block_hash,
                "blockNumber":format!("0x{:x}",logs[0].block_number.unwrap()),
                "from":A,"to":B,"contractAddress":null,"cumulativeGasUsed":"0x1","gasUsed":"0x1",
                "effectiveGasPrice":"0x1","logs":logs,"status":"0x1","type":"0x0",
                "logsBloom": format!("0x{}", "00".repeat(256))})
        }
        "eth_getLogs" => {
            assert!(
                p[0].get("address").is_none(),
                "default address must never restrict the scan"
            );
            let from = height(&p[0]["fromBlock"]);
            let to = height(&p[0]["toBlock"]);
            let topic = p[0]["topics"].get(1).filter(|t| !t.is_null());
            let logs = state
                .logs
                .iter()
                .filter(|l| {
                    state.ignore_log_filter
                        || ((from..=to).contains(&l.block_number.unwrap())
                            && topic.is_none_or(|t| *t == json!(l.topics()[1])))
                })
                .collect::<Vec<_>>();
            if state.log_cap.is_some_and(|cap| logs.len() > cap) {
                return Json(json!({"jsonrpc":"2.0","id":request["id"],
                    "error":{"code":-32603,"message":"query returned more than 10000 results"}}));
            }
            json!(logs)
        }
        "eth_call" => {
            if state.fail_history || state.fail_history_at == Some(height(&p[1])) {
                return Json(
                    json!({"jsonrpc":"2.0", "id":request["id"], "error":{"code":-32000,"message":"historical state unavailable"}}),
                );
            }
            let block = height(&p[1]);
            let to: Address = serde_json::from_value(p[0]["to"].clone()).unwrap();
            let raw = p[0]
                .get("input")
                .or_else(|| p[0].get("data"))
                .unwrap()
                .as_str()
                .unwrap();
            let data = hex::decode(raw.trim_start_matches("0x")).unwrap();
            let expected_registry = if block < 130 { REGISTRY } else { REPLACEMENT };
            let encoded = if to == resolver::CHAIN_INFO_PRECOMPILE {
                let call = IChainInfo::get_outbox_discovery_addressCall::abi_decode(&data).unwrap();
                assert_eq!(call.chainKey, 7);
                (expected_registry, true).abi_encode()
            } else {
                assert_eq!(
                    to, expected_registry,
                    "must use the registry at the message block, not today's registry"
                );
                let call = IOutboxDiscovery::isActiveOutboxCall::abi_decode(&data).unwrap();
                assert_eq!(call.chainKey, 7);
                // A is removed at 120, re-registered at 125, and absent from the new registry.
                // B is active throughout (it became default at 105, which is irrelevant here).
                let active = call.outbox == B
                    || (call.outbox == A
                        && ((100..120).contains(&block) || (125..130).contains(&block)));
                active.abi_encode()
            };
            json!(format!("0x{}", hex::encode(encoded)))
        }
        other => panic!("unexpected RPC {other}"),
    };
    Json(json!({"jsonrpc":"2.0", "id":request["id"], "result":result}))
}

async fn poll(rpc: &Rpc, last_seen: &mut u64) -> anyhow::Result<Vec<listener::IndexedMessage>> {
    let provider = ProviderBuilder::new().connect_http(rpc.url.clone());
    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    listener::poll_once(
        &provider,
        &route(),
        &listener::FinalityPolicy::Depth(0),
        &mut listener::FinalityTracker::new(std::time::Instant::now()),
        last_seen,
        &tx,
    )
    .await?;
    drop(tx);
    let mut messages = vec![];
    while let Some(message) = rx.recv().await {
        messages.push(message);
    }
    Ok(messages)
}

#[tokio::test]
async fn default_rotation_drains_all_outboxes_and_enforces_historical_removal() {
    let rpc = Rpc::new(vec![
        message(A, 101, 1),
        message(B, 110, 2),
        message(A, 119, 3),
        message(A, 120, 4),
        message(A, 124, 5),
        message(A, 125, 6),
        message(A, 132, 7),
        message(B, 135, 8),
        message(ROGUE, 136, 9),
    ])
    .await;
    let mut cursor = 100;
    let messages = poll(&rpc, &mut cursor).await.unwrap();
    assert_eq!(
        messages.iter().map(|m| m.message_id[0]).collect::<Vec<_>>(),
        [1, 2, 3, 6, 8]
    );
    assert_eq!(cursor, 140);
    assert_eq!(
        messages[2].outbox, A,
        "old non-default Outbox still drains before removal"
    );
    assert!(messages
        .iter()
        .all(|m| m.destination_chain_key == route().destination_chain_key));
}

#[tokio::test]
async fn history_rpc_failure_never_advances_cursor_and_recovery_replays_range() {
    let rpc = Rpc::new(vec![message(A, 119, 3)]).await;
    let mut cursor = 100;
    rpc.state.lock().unwrap().fail_history = true;
    assert!(poll(&rpc, &mut cursor).await.is_err());
    assert_eq!(cursor, 100);
    rpc.state.lock().unwrap().fail_history = false;
    assert_eq!(
        poll(&rpc, &mut cursor).await.unwrap()[0].message_id,
        B256::repeat_byte(3)
    );
    assert_eq!(cursor, 140);
}

#[tokio::test]
async fn historical_reobservation_survives_removal_and_registry_replacement() {
    let rpc = Rpc::new(vec![
        message(A, 119, 3),
        message(A, 120, 4),
        message(ROGUE, 119, 5),
    ])
    .await;
    let provider = ProviderBuilder::new().connect_http(rpc.url.clone());
    for (id, block, expected) in [(3, 119, true), (4, 120, false), (5, 119, false)] {
        let request = ReobservationRequest {
            chain_key: 7,
            message_id: [id; 32],
            tx_hash: [id; 32],
            block_height: block,
        };
        let found = reobservation::reobserve(&provider, &route(), 3, &request)
            .await
            .unwrap();
        assert_eq!(found.is_some(), expected);
        if let Some(found) = found {
            assert_eq!(found.outbox, A);
        }
    }
}

#[tokio::test]
async fn out_of_range_or_removed_logs_do_not_advance_cursor() {
    for mut log in [message(A, 99, 1), message(A, 141, 1), message(A, 119, 1)] {
        if log.block_number == Some(119) {
            log.removed = true;
        }
        let rpc = Rpc::new(vec![log]).await;
        rpc.state.lock().unwrap().ignore_log_filter = true;
        let mut cursor = 100;
        assert!(poll(&rpc, &mut cursor).await.is_err());
        assert_eq!(cursor, 100);
    }
}

#[tokio::test]
async fn capped_ranges_split_and_single_block_spam_falls_back_to_receipts() {
    let rpc = Rpc::new(vec![
        message(A, 119, 1),
        message(ROGUE, 119, 2),
        message(B, 135, 3),
    ])
    .await;
    rpc.state.lock().unwrap().log_cap = Some(1);
    let mut cursor = 100;
    let found = poll(&rpc, &mut cursor).await.unwrap();
    assert_eq!(
        found.iter().map(|m| m.message_id[0]).collect::<Vec<_>>(),
        [1, 3]
    );
    assert_eq!(cursor, 140);
    assert!(rpc
        .state
        .lock()
        .unwrap()
        .calls
        .iter()
        .any(|c| c["method"] == "eth_getTransactionReceipt"));
}

#[tokio::test]
async fn split_scan_commits_drained_prefix_before_a_later_history_failure() {
    let rpc = Rpc::new(vec![message(A, 119, 1), message(B, 135, 2)]).await;
    {
        let mut state = rpc.state.lock().unwrap();
        state.log_cap = Some(1);
        state.fail_history_at = Some(135);
    }
    let mut cursor = 100;
    assert!(poll(&rpc, &mut cursor).await.is_err());
    assert_eq!(
        cursor, 120,
        "successfully drained first split must be committed independently"
    );
    rpc.state.lock().unwrap().fail_history_at = None;
    let messages = poll(&rpc, &mut cursor).await.unwrap();
    assert_eq!(
        messages.iter().map(|m| m.message_id[0]).collect::<Vec<_>>(),
        [2]
    );
    assert_eq!(cursor, 140);
}
