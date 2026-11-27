//! Pre-Byzantium Ethereum mainnet receipts carry no consensus status (EIP-658 came with
//! Byzantium), and the V1 leaf encodes one. Erigon/Reth-style nodes serve the real execution
//! status; Geth-style nodes serve only the post-state root, which alloy coerces to success. These
//! tests pin that `OrderedBlock::try_from_fetched_block` accepts the former, rejects the latter
//! as a per-peer payload problem (so the fetch moves on to the next endpoint), and leaves every
//! other chain and height alone.
//!
//! Fixture: Ethereum mainnet block 151000. Its only transaction,
//! 0xe5e48d1c5bf232ef4064879dd25dd7e5e38eb0bcc0bc733a0a1f489650010867, burned its whole gas limit
//! (150000) and failed: status-reporting nodes serve `"status": "0x0"`, root-only nodes serve the
//! post-state root below.

use alloy::rpc::types::{Block, TransactionReceipt};
use attestcoin_abi_encoding::common::EncodingVersion;
use eth::{Client, Error, OrderedBlock};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

const NUMBER: u64 = 151_000;
const BYZANTIUM: u64 = 4_370_000;
const POST_STATE_ROOT: &str = "0x5c68a77c058bc2d55736a83fcb51f5bc65a6932e8c3f64b323a75d859236e87e";

const BLOCK: &str = r#"{"difficulty":"0x56d2a469020","extraData":"0xd783010002844765746887676f312e342e32856c696e7578","gasLimit":"0x2fefd8","gasUsed":"0x249f0","hash":"0x9d81373b556eba5c130ff65f91350613d63e4897415d8a0a88c1e5bcad0468e0","logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","miner":"0x790b8a3ce86e707ed0ed32bf89b3269692a23cc1","mixHash":"0x353de6275ac9e149a81bb47cbbc0be3938550c05245cd62491f30e398112cf7c","nonce":"0xb41dc66a911bafe4","number":"0x24dd8","parentHash":"0x883a76df3751ca65dbc018c7f90976001a1bdd65c0d9d22ccf7de1873ce93628","receiptsRoot":"0xcda12ba74bb4079d08184e3a07a6454ff7d25f50f40ccb33515c5682dd37d03e","sha3Uncles":"0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347","size":"0x30d","stateRoot":"0x33be03c0fe64aa17838f6258f24240eb26d5b31e60f73696a112f477b6dc7f41","timestamp":"0x55df0af4","transactions":[{"blockHash":"0x9d81373b556eba5c130ff65f91350613d63e4897415d8a0a88c1e5bcad0468e0","blockNumber":"0x24dd8","blockTimestamp":"0x55df0af4","from":"0xa9ac1233699bdae25abebae4f9fb54dbb1b44700","gas":"0x249f0","gasPrice":"0xba43b7400","hash":"0xe5e48d1c5bf232ef4064879dd25dd7e5e38eb0bcc0bc733a0a1f489650010867","input":"0x92c04524000000000000000000000000000c764b4ab0c4c34bcf8aad7c92a1e6c5d222220000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000000c5365636f6e64206167656e740000000000000000000000000000000000000000","nonce":"0x5f","to":"0x03a81faf699715cf9563d03565d9414ada137053","transactionIndex":"0x0","value":"0x0","type":"0x0","v":"0x1c","r":"0x757301d9f9f0dab69b2c62a77a16a00ba3ab7e76a600430b6007feec47a4e2e9","s":"0x1d23fd1bf513935f88a01a572512e51f800aa8cc8a0383c6e7089c6346a9bca3"}],"transactionsRoot":"0x50e20409f466fc86e0dad968b91e4daf928d96f11cdf5b6fd3a7f653bf1329a0","uncles":[]}"#;

/// The receipt as an Erigon/Reth-style node serves it: real status, no root.
const RECEIPT_WITH_STATUS: &str = r#"{"type":"0x0","status":"0x0","cumulativeGasUsed":"0x249f0","logs":[],"logsBloom":"0x00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000","transactionHash":"0xe5e48d1c5bf232ef4064879dd25dd7e5e38eb0bcc0bc733a0a1f489650010867","transactionIndex":"0x0","blockHash":"0x9d81373b556eba5c130ff65f91350613d63e4897415d8a0a88c1e5bcad0468e0","blockNumber":"0x24dd8","gasUsed":"0x249f0","effectiveGasPrice":"0xba43b7400","from":"0xa9ac1233699bdae25abebae4f9fb54dbb1b44700","to":"0x03a81faf699715cf9563d03565d9414ada137053","contractAddress":null}"#;

fn block_at(number: u64) -> Block {
    let mut v: Value = serde_json::from_str(BLOCK).unwrap();
    let hex = format!("{number:#x}");
    v["number"] = hex.clone().into();
    v["transactions"][0]["blockNumber"] = hex.into();
    serde_json::from_value(v).unwrap()
}

fn receipt_with_status(number: u64) -> Value {
    let mut v: Value = serde_json::from_str(RECEIPT_WITH_STATUS).unwrap();
    v["blockNumber"] = format!("{number:#x}").into();
    v
}

/// The same receipt as a Geth-style node serves it: post-state root, no status.
fn receipt_with_root(number: u64) -> Value {
    let mut v = receipt_with_status(number);
    let obj = v.as_object_mut().unwrap();
    obj.remove("status");
    obj.insert("root".into(), POST_STATE_ROOT.into());
    v
}

fn receipts(v: Value) -> Vec<TransactionReceipt> {
    vec![serde_json::from_value(v).unwrap()]
}

#[test]
fn real_status_is_accepted_and_kept() {
    let ob = OrderedBlock::try_from_fetched_block(
        1,
        block_at(NUMBER),
        receipts(receipt_with_status(NUMBER)),
        NUMBER,
        EncodingVersion::V1,
    )
    .expect("a status-reporting peer must be accepted");
    assert_eq!(ob.items().len(), 1);
    assert!(
        !ob.items()[0].rx().inner.status(),
        "the failed transaction must stay failed"
    );
}

#[test]
fn root_only_receipts_are_rejected_for_the_next_endpoint() {
    let err = OrderedBlock::try_from_fetched_block(
        1,
        block_at(NUMBER),
        receipts(receipt_with_root(NUMBER)),
        NUMBER,
        EncodingVersion::V1,
    )
    .expect_err("a root-only peer would encode the failed transaction as successful");

    assert!(
        matches!(err, Error::PreByzantiumStatusMissing(NUMBER)),
        "got {err:?}"
    );
    // It must reach the fallback walk, not the reconnect path.
    assert!(err.inconsistent_block_payload_for_fallback());
    assert_eq!(err.inconsistent_block_number_hint(), Some(NUMBER));
}

#[test]
fn root_only_receipts_are_fine_from_byzantium_on() {
    // From Byzantium on the post-state root never appears on mainnet; when it does (here, the same
    // canonical receipt relabelled), the header receipt-root check is what governs, and the
    // canonical root form reproduces the header root.
    OrderedBlock::try_from_fetched_block(
        1,
        block_at(BYZANTIUM),
        receipts(receipt_with_root(BYZANTIUM)),
        BYZANTIUM,
        EncodingVersion::V1,
    )
    .expect("the status check is limited to pre-Byzantium mainnet blocks");
}

#[test]
fn root_only_receipts_are_fine_on_other_chains() {
    OrderedBlock::try_from_fetched_block(
        11_155_111,
        block_at(NUMBER),
        receipts(receipt_with_root(NUMBER)),
        NUMBER,
        EncodingVersion::V1,
    )
    .expect("the status check is limited to Ethereum mainnet");
}

/// HTTP JSON-RPC node serving block 151000, with its receipt in the form `receipt` builds.
struct Mock {
    url: String,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Mock {
    fn start(receipt: fn(u64) -> Value) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let thread = thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                stream.set_nonblocking(false).unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                let mut length = 0;
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some((k, v)) = line.split_once(':') {
                        if k.eq_ignore_ascii_case("content-length") {
                            length = v.trim().parse::<usize>().unwrap();
                        }
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let request: Value = serde_json::from_slice(&body).unwrap();
                let result = match request["method"].as_str().unwrap() {
                    "eth_chainId" => json!("0x1"),
                    "eth_blockNumber" => json!("0x1000000"),
                    "eth_getBlockByNumber" => serde_json::from_str(BLOCK).unwrap(),
                    "eth_getBlockReceipts" => json!([receipt(NUMBER)]),
                    _ => Value::Null,
                };
                let out = serde_json::to_vec(
                    &json!({"jsonrpc": "2.0", "id": request["id"], "result": result}),
                )
                .unwrap();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    out.len()
                )
                .unwrap();
                stream.write_all(&out).unwrap();
            }
        });
        Self {
            url,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}

#[tokio::test]
async fn fetch_moves_past_a_root_only_primary_to_a_status_fallback() {
    let root_only = Mock::start(receipt_with_root);
    let with_status = Mock::start(receipt_with_status);
    let client = Client::new_with_fallbacks(&root_only.url, &[with_status.url.clone()], None)
        .await
        .unwrap();

    let ob = match tokio::time::timeout(
        Duration::from_secs(10),
        client.get_block(NUMBER, EncodingVersion::V1),
    )
    .await
    {
        Ok(Ok(ob)) => ob,
        Ok(Err(user::Interrupt::Cont(e))) => {
            panic!("the status-reporting fallback must serve the block: {e:?}")
        }
        Ok(Err(user::Interrupt::Stop)) => panic!("not a shutdown"),
        Err(_) => panic!("the fetch did not move on to the fallback"),
    };
    assert!(!ob.items()[0].rx().inner.status());
}
