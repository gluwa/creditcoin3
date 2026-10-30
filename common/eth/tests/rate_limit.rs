//! Request pacing and rate-limit handling against a tiny in-process JSON-RPC server: a paced
//! client spaces its calls out, and a block fetch that is answered with HTTP 429 holds off and
//! retries instead of failing or hammering the provider.
use attestcoin_abi_encoding::common::EncodingVersion;
use eth::Client;
use serde_json::{json, Value};
use std::{
    io::{BufRead, Read, Write},
    net::TcpListener,
    num::NonZeroU32,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const ZERO32: &str = "0x0000000000000000000000000000000000000000000000000000000000000000";

/// An empty block: no transactions, so the fetch skips the header-root check.
fn empty_block(number: u64) -> Value {
    json!({
        "hash": format!("0x{number:064x}"), "parentHash": ZERO32, "sha3Uncles": ZERO32,
        "miner": "0x0000000000000000000000000000000000000000",
        "stateRoot": ZERO32, "transactionsRoot": ZERO32, "receiptsRoot": ZERO32,
        "logsBloom": format!("0x{}", "00".repeat(256)),
        "difficulty": "0x0", "number": format!("{number:#x}"), "gasLimit": "0x1c9c380",
        "gasUsed": "0x0", "timestamp": "0x64", "extraData": "0x", "mixHash": ZERO32,
        "nonce": "0x0000000000000000", "baseFeePerGas": "0x1",
        "transactions": [], "uncles": []
    })
}

/// Write an HTTP response, ignoring failures: once a block call is refused the client drops the
/// paired receipts call, so its connection may already be closed when the answer is written.
fn respond(stream: &mut std::net::TcpStream, status: &str, body: &[u8]) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(body);
}

struct Mock {
    url: String,
    /// Arrival time of every request other than `eth_chainId`.
    arrivals: Arc<Mutex<Vec<Instant>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Mock {
    /// Serves `eth_chainId`, `eth_blockNumber` (100) and empty blocks with no receipts. The first
    /// `rate_limited` `eth_getBlockByNumber` requests are refused with HTTP 429. Only that call is
    /// refused because the receipts call of the same sweep may be cancelled before it is sent
    /// once the block call fails, which would make a per-call count racy.
    fn start(rate_limited: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let arrivals = Arc::new(Mutex::new(Vec::new()));
        let seen = arrivals.clone();
        let refused = Arc::new(AtomicUsize::new(0));
        let thread = thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(_) => {
                        thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                };
                stream.set_nonblocking(false).unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let mut length = 0;
                loop {
                    line.clear();
                    if reader.read_line(&mut line).is_err() || line == "\r\n" {
                        break;
                    }
                    if let Some((k, v)) = line.split_once(':') {
                        if k.eq_ignore_ascii_case("content-length") {
                            length = v.trim().parse::<usize>().unwrap_or(0);
                        }
                    }
                }
                let mut body = vec![0; length];
                if reader.read_exact(&mut body).is_err() {
                    continue;
                }
                let request: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let method = request["method"].as_str().unwrap_or_default();
                if method != "eth_chainId" {
                    seen.lock().unwrap().push(Instant::now());
                }
                if method == "eth_getBlockByNumber"
                    && refused.fetch_add(1, Ordering::SeqCst) < rate_limited
                {
                    let out = b"{\"error\":\"rate limited\"}";
                    respond(&mut stream, "429 Too Many Requests", out);
                    continue;
                }
                let result = match method {
                    "eth_chainId" => json!("0x539"),
                    "eth_blockNumber" => json!("0x64"),
                    "eth_getBlockByNumber" => {
                        let number = request["params"][0]
                            .as_str()
                            .and_then(|n| u64::from_str_radix(n.trim_start_matches("0x"), 16).ok())
                            .unwrap_or(0);
                        empty_block(number)
                    }
                    "eth_getBlockReceipts" => json!([]),
                    _ => Value::Null,
                };
                let out = serde_json::to_vec(
                    &json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
                )
                .unwrap();
                respond(&mut stream, "200 OK", &out);
            }
        });
        Self {
            url,
            arrivals,
            stop,
            thread: Some(thread),
        }
    }

    fn arrivals(&self) -> Vec<Instant> {
        self.arrivals.lock().unwrap().clone()
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[tokio::test]
async fn paced_client_spaces_requests_out() {
    let mock = Mock::start(0);
    // 20 rps = one request every 50 ms.
    let client = Client::new(&mock.url, None)
        .await
        .unwrap()
        .with_rate_limit(NonZeroU32::new(20).unwrap());

    for _ in 0..5 {
        client.get_last_block().await.unwrap();
    }

    let arrivals = mock.arrivals();
    assert_eq!(arrivals.len(), 5);
    let span = arrivals[4] - arrivals[0];
    assert!(
        span >= Duration::from_millis(190),
        "5 requests at 20 rps took only {span:?}"
    );
}

#[tokio::test]
async fn clones_share_one_budget() {
    let mock = Mock::start(0);
    let client = Client::new(&mock.url, None)
        .await
        .unwrap()
        .with_rate_limit(NonZeroU32::new(20).unwrap());

    let tasks: Vec<_> = (0..6)
        .map(|_| {
            let client = client.clone();
            tokio::spawn(async move { client.get_last_block().await.unwrap() })
        })
        .collect();
    for task in tasks {
        task.await.unwrap();
    }

    let arrivals = mock.arrivals();
    let span = *arrivals.iter().max().unwrap() - *arrivals.iter().min().unwrap();
    assert!(
        span >= Duration::from_millis(240),
        "6 requests at 20 rps took only {span:?}"
    );
}

#[tokio::test]
async fn rate_limited_block_fetch_holds_off_and_succeeds() {
    // Two refused sweeps hold off 1 s, then 2 s, before the third sweep is served. An ordinary
    // failed sweep would back off 10 s, so the timing shows the rate-limit path was taken.
    let mock = Mock::start(2);
    let client = Client::new(&mock.url, None).await.unwrap();

    let start = Instant::now();
    let block = client
        .get_block(42, EncodingVersion::V1)
        .await
        .unwrap_or_else(|e| panic!("block fetch failed: {e}"));
    let elapsed = start.elapsed();

    assert_eq!(block.number(), 42);
    assert!(
        elapsed >= Duration::from_secs(3),
        "held off only {elapsed:?}"
    );
    // Well under the 10 s backoff an ordinary failed sweep waits.
    assert!(elapsed < Duration::from_secs(8), "took {elapsed:?}");
}
