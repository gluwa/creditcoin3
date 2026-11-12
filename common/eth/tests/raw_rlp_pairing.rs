//! The raw-RLP fetch asks for receipts by the hash of the block it received, so a load balancer
//! that answers the two calls from opposite sides of a reorg cannot pair a block with another
//! fork's receipts. The mock serves pre-Byzantium Ethereum mainnet, where the receipt-root check
//! is skipped and nothing else would catch a same-length swap.
use alloy::{
    consensus::{
        proofs::{calculate_receipt_root, calculate_transaction_root},
        Block as ConsensusBlock, BlockBody, Header as ConsensusHeader, Receipt, ReceiptEnvelope,
        SignableTransaction, TxEnvelope, TxLegacy,
    },
    eips::eip2718::Encodable2718,
    primitives::{address, hex, TxKind, B256, U256},
    signers::{local::PrivateKeySigner, SignerSync},
};
use attestcoin_abi_encoding::common::EncodingVersion;
use eth::{BlockFetchMode, Client};
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

fn legacy_receipt(status: bool) -> ReceiptEnvelope {
    ReceiptEnvelope::Legacy(
        Receipt {
            status: status.into(),
            cumulative_gas_used: 21_000,
            logs: vec![],
        }
        .with_bloom(),
    )
}

fn raw_receipt(status: bool) -> String {
    hex::encode_prefixed(legacy_receipt(status).encoded_2718())
}

/// One-transfer block at [`NUMBER`] whose header commits to a successful receipt.
fn raw_block() -> (String, B256) {
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
        receipts_root: calculate_receipt_root(&[legacy_receipt(true)]),
        ..Default::default()
    };
    let hash = header.hash_slow();
    let block = ConsensusBlock {
        header,
        body: BlockBody {
            transactions: txs,
            ommers: vec![],
            withdrawals: None,
        },
    };
    (hex::encode_prefixed(alloy::rlp::encode(&block)), hash)
}

/// Serves `raw_block` and answers `debug_getRawReceipts` with this block's (successful) receipt
/// when asked by its hash, an empty list for any other hash, and the other fork's (failed)
/// receipt when asked by height.
struct Mock {
    url: String,
    by_hash: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Mock {
    fn start(raw_block: String, hash: B256) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let by_hash = Arc::new(AtomicUsize::new(0));
        let hash_reads = by_hash.clone();
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
                let result = match request["method"].as_str().unwrap() {
                    "eth_chainId" => json!("0x1"),
                    "debug_getRawBlock" => json!(raw_block),
                    "debug_getRawReceipts" => match &request["params"][0] {
                        Value::Object(id) => {
                            hash_reads.fetch_add(1, Ordering::SeqCst);
                            if id["blockHash"].as_str() == Some(&format!("{hash:?}")) {
                                json!([raw_receipt(true)])
                            } else {
                                json!([])
                            }
                        }
                        _ => json!([raw_receipt(false)]),
                    },
                    m => panic!("unexpected method {m}"),
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
            by_hash,
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
async fn receipts_are_fetched_by_the_hash_of_the_fetched_block() {
    let (raw_block, hash) = raw_block();
    let mock = Mock::start(raw_block, hash);
    let client = Client::new(&mock.url, None)
        .await
        .unwrap()
        .with_fetch_mode(BlockFetchMode::RawRlp);

    let block = client.get_block(NUMBER, EncodingVersion::V1).await.unwrap();

    assert_eq!(block.hash(), hash);
    assert!(
        block.items()[0].eth_rx().unwrap().status(),
        "the receipt must be this block's, not the other fork's served by height"
    );
    assert_eq!(mock.by_hash.load(Ordering::SeqCst), 1);
}
