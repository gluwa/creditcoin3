//! An endpoint that answers with more than [`eth::HTTP_MAX_RESPONSE_SIZE`] bytes must be refused
//! without the client reading the reply, and a block fetch must fail over to the next endpoint.
use alloy::{
    consensus::{
        proofs::{calculate_receipt_root, calculate_transaction_root},
        Block as ConsensusBlock, BlockBody, Header as ConsensusHeader, Receipt, ReceiptEnvelope,
        SignableTransaction, TxEnvelope, TxLegacy,
    },
    eips::eip2718::Encodable2718,
    primitives::{address, hex, TxKind, U256},
    signers::{local::PrivateKeySigner, SignerSync},
};
use attestcoin_abi_encoding::common::EncodingVersion;
use eth::{BlockFetchMode, Client, HTTP_MAX_RESPONSE_SIZE};
use serde_json::{json, Value};
use std::{
    io::{BufRead, Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

const NUMBER: u64 = 1_000_000;

fn legacy_receipt() -> ReceiptEnvelope {
    ReceiptEnvelope::Legacy(
        Receipt {
            status: true.into(),
            cumulative_gas_used: 21_000,
            logs: vec![],
        }
        .with_bloom(),
    )
}

/// A one-transfer block at [`NUMBER`] whose header commits to [`legacy_receipt`], hex encoded as
/// `debug_getRawBlock` returns it, and that receipt.
fn raw_block_and_receipt() -> (String, String) {
    let signer = PrivateKeySigner::random();
    let tx = TxLegacy {
        chain_id: Some(1),
        nonce: 0,
        gas_price: 20_000_000_000,
        gas_limit: 21_000,
        to: TxKind::Call(address!("000000000000000000000000000000000000dEaD")),
        value: U256::from(1u64),
        input: Default::default(),
    };
    let sig = signer.sign_hash_sync(&tx.signature_hash()).unwrap();
    let txs = vec![TxEnvelope::Legacy(tx.into_signed(sig))];
    let header = ConsensusHeader {
        number: NUMBER,
        gas_used: 21_000,
        gas_limit: 8_000_000,
        transactions_root: calculate_transaction_root(&txs),
        receipts_root: calculate_receipt_root(&[legacy_receipt()]),
        ..Default::default()
    };
    let block = ConsensusBlock {
        header,
        body: BlockBody {
            transactions: txs,
            ommers: vec![],
            withdrawals: None,
        },
    };
    (
        hex::encode_prefixed(alloy::rlp::encode(&block)),
        hex::encode_prefixed(legacy_receipt().encoded_2718()),
    )
}

/// How a mock node answers a block request.
#[derive(Clone)]
enum Node {
    /// Serves the block and its receipt.
    Honest {
        raw_block: String,
        raw_receipt: String,
    },
    /// Answers the block request by declaring a reply one byte over the cap and keeps the
    /// connection open without sending the rest. Only the declared length can make the client
    /// refuse; without a cap it would wait on the body until its request timeout.
    Oversized,
}

/// HTTP JSON-RPC node on a loopback port. `block_requests` counts `debug_getRawBlock` calls.
struct Mock {
    url: String,
    block_requests: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Mock {
    fn start(node: Node) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let block_requests = Arc::new(AtomicUsize::new(0));
        let counted = block_requests.clone();
        let thread = thread::spawn(move || {
            // Connections left open on purpose: see `Node::Oversized`.
            let mut held = Vec::new();
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
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
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
                let method = request["method"].as_str().unwrap();

                if method == "debug_getRawBlock" {
                    counted.fetch_add(1, Ordering::SeqCst);
                    if matches!(node, Node::Oversized) {
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{{\"jsonrpc\"",
                            HTTP_MAX_RESPONSE_SIZE + 1
                        )
                        .unwrap();
                        held.push(stream);
                        continue;
                    }
                }
                let result = match (method, &node) {
                    ("eth_chainId", _) => json!("0x1"),
                    ("debug_getRawBlock", Node::Honest { raw_block, .. }) => json!(raw_block),
                    ("debug_getRawReceipts", Node::Honest { raw_receipt, .. }) => {
                        json!([raw_receipt])
                    }
                    (m, _) => panic!("unexpected method {m}"),
                };
                let out = serde_json::to_vec(
                    &json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
                )
                .unwrap();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", out.len()).unwrap();
                stream.write_all(&out).unwrap();
            }
        });
        Self {
            url,
            block_requests,
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
async fn a_block_reply_over_the_cap_fails_over_to_the_next_endpoint() {
    let (raw_block, raw_receipt) = raw_block_and_receipt();
    let hostile = Mock::start(Node::Oversized);
    let honest = Mock::start(Node::Honest {
        raw_block,
        raw_receipt,
    });
    let client = Client::new_with_fallbacks(&hostile.url, &[honest.url.clone()], None)
        .await
        .unwrap()
        .with_fetch_mode(BlockFetchMode::RawRlp);

    let block = tokio::time::timeout(
        Duration::from_secs(10),
        client.get_block(NUMBER, EncodingVersion::V1),
    )
    .await
    .expect("failing over must not back off")
    .expect("the honest endpoint serves the block");

    assert_eq!(block.number(), NUMBER);
    assert_eq!(block.items().len(), 1);
    assert_eq!(hostile.block_requests.load(Ordering::SeqCst), 1);
    assert_eq!(honest.block_requests.load(Ordering::SeqCst), 1);
}
