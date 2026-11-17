//! In JSON fetch mode a provider that reports a transaction sender its signature does not
//! recover to is discarded, and the fetch is served by the next endpoint instead.
use alloy::{
    consensus::{
        proofs::{calculate_receipt_root, calculate_transaction_root},
        Block as ConsensusBlock, BlockBody, Header as ConsensusHeader, Receipt, ReceiptEnvelope,
        SignableTransaction, TxEnvelope, TxLegacy,
    },
    eips::eip2718::Encodable2718,
    primitives::{address, Address, Bytes, TxKind, U256},
    signers::{local::PrivateKeySigner, SignerSync},
};
use attestcoin_abi_encoding::common::EncodingVersion;
use eth::{rpc_block_from_raw, Client};
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

/// The JSON-RPC `eth_getBlockByNumber` and `eth_getBlockReceipts` results for a one-transfer
/// block whose header commits to its transaction and receipt, plus the real sender.
fn json_block() -> (Value, Value, Address) {
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
    let receipt = ReceiptEnvelope::Legacy(
        Receipt {
            status: true.into(),
            cumulative_gas_used: 21_000,
            logs: vec![],
        }
        .with_bloom(),
    );
    let header = ConsensusHeader {
        number: NUMBER,
        gas_used: 21_000,
        gas_limit: 8_000_000,
        transactions_root: calculate_transaction_root(&txs),
        receipts_root: calculate_receipt_root(std::slice::from_ref(&receipt)),
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
    let (block, receipts) = rpc_block_from_raw(
        NUMBER,
        &alloy::rlp::encode(&block),
        &[Bytes::from(receipt.encoded_2718())],
    )
    .unwrap();
    (
        serde_json::to_value(block).unwrap(),
        serde_json::to_value(receipts).unwrap(),
        signer.address(),
    )
}

/// HTTP JSON-RPC node serving one block and its receipts.
struct Mock {
    url: String,
    block_reads: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Mock {
    fn start(block: Value, receipts: Value) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let block_reads = Arc::new(AtomicUsize::new(0));
        let reads = block_reads.clone();
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
                    "eth_getBlockByNumber" => {
                        reads.fetch_add(1, Ordering::SeqCst);
                        block.clone()
                    }
                    "eth_getBlockReceipts" => receipts.clone(),
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
            block_reads,
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
async fn a_provider_reporting_a_wrong_sender_is_skipped_for_the_next_endpoint() {
    let (block, receipts, sender) = json_block();
    let mut forged = block.clone();
    forged["transactions"][0]["from"] = json!("0x00000000000000000000000000000000000000aa");

    let primary = Mock::start(forged, receipts.clone());
    let fallback = Mock::start(block, receipts);
    let client = Client::new_with_fallbacks(&primary.url, &[fallback.url.clone()], None)
        .await
        .unwrap();

    let fetched = client.get_block(NUMBER, EncodingVersion::V1).await.unwrap();

    let eth::TxRx::Ethereum { tx, .. } = &fetched.items()[0] else {
        panic!("expected an Ethereum transaction");
    };
    assert_eq!(tx.inner.signer(), sender);
    assert_eq!(primary.block_reads.load(Ordering::SeqCst), 1);
    assert_eq!(fallback.block_reads.load(Ordering::SeqCst), 1);
}
