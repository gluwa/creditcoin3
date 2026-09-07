use eth::{ChainFamily, Client};
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
use usc_abi_encoding::common::EncodingVersion;

struct RpcMock {
    url: String,
    reads: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl RpcMock {
    fn start(chain_id: u64, block: Value, receipts: Value) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let reads = Arc::new(AtomicUsize::new(0));
        let read_count = reads.clone();
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
                // Accepted sockets inherit O_NONBLOCK on macOS; requests are read synchronously.
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
                    "eth_chainId" => json!(format!("0x{chain_id:x}")),
                    "eth_getBlockByNumber" => {
                        read_count.fetch_add(1, Ordering::SeqCst);
                        block.clone()
                    }
                    "eth_getBlockReceipts" => {
                        read_count.fetch_add(1, Ordering::SeqCst);
                        receipts.clone()
                    }
                    m => panic!("unexpected method {m}"),
                };
                let output = serde_json::to_vec(
                    &json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
                )
                .unwrap();
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",output.len()).unwrap();
                stream.write_all(&output).unwrap();
            }
        });
        Self {
            url,
            reads,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for RpcMock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}
fn fixture() -> (Value, Value) {
    (
        serde_json::from_str(include_str!("fixtures/base_sepolia_46388021_block.json")).unwrap(),
        serde_json::from_str(include_str!("fixtures/base_sepolia_46388021_receipts.json")).unwrap(),
    )
}

#[tokio::test]
async fn bad_primary_deposit_and_receipt_payloads_use_the_healthy_fallback() {
    let (block, receipts) = fixture();
    for case in 0..4 {
        let (mut bad_block, mut bad_receipts) = (block.clone(), receipts.clone());
        match case {
            0 => bad_receipts[1]["type"] = json!("0x0"),
            1 => bad_block["transactions"][0]["nonce"] = json!("0x0"),
            2 => bad_block["transactions"][0]["value"] = json!("0x1"),
            _ => bad_receipts[0]["depositReceiptVersion"] = json!("invalid"),
        }
        let primary = RpcMock::start(999999, bad_block, bad_receipts);
        let backup = RpcMock::start(999999, block.clone(), receipts.clone());
        let client =
            Client::new_with_fallbacks(&primary.url, std::slice::from_ref(&backup.url), None)
                .await
                .unwrap()
                .with_chain_family_override(Some(ChainFamily::OpStack));
        let fetched = client
            .get_block(46388021, EncodingVersion::V1)
            .await
            .unwrap();
        assert_eq!(fetched.items().len(), 11);
        assert!(
            backup.reads.load(Ordering::SeqCst) >= 2,
            "case {case} skipped the fallback"
        );
    }
}

#[tokio::test]
async fn unlisted_rollup_override_survives_reconnect_and_inference_still_fails_fast() {
    let (block, receipts) = fixture();
    let primary = RpcMock::start(999999, block.clone(), receipts.clone());
    let backup = RpcMock::start(999999, block, receipts);
    let client = Client::new_with_fallbacks(&primary.url, std::slice::from_ref(&backup.url), None)
        .await
        .unwrap();
    let inferred = client.clone().with_chain_family_override(None);
    assert_eq!(inferred.chain_family(), ChainFamily::Ethereum);
    let error = inferred
        .get_block(46388021, EncodingVersion::V1)
        .await
        .unwrap_err();
    assert!(format!("{error:?}").contains("UnsupportedTransactionType"));
    assert_eq!(backup.reads.load(Ordering::SeqCst), 0);

    let mut configured = client.with_chain_family_override(Some(ChainFamily::OpStack));
    let first = configured
        .get_block(46388021, EncodingVersion::V1)
        .await
        .unwrap();
    configured.reconnect().await.unwrap();
    assert_eq!(configured.chain_family(), ChainFamily::OpStack);
    let reconnected = configured
        .get_block(46388021, EncodingVersion::V1)
        .await
        .unwrap();
    assert_eq!(
        eth::simple_merkle_tree(&first).root(),
        eth::simple_merkle_tree(&reconnected).root()
    );
}
