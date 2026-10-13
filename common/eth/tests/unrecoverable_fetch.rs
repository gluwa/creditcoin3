//! A block that no retry can fetch (the node lacks the method, or serves bytes this client cannot
//! decode) must fail the fetch at once instead of backing off through the full retry budget, and
//! the startup check must report the endpoint as unusable for that fetch mode.
use eth::{BlockFetchMode, Client, Error};
use serde_json::{json, Value};
use std::{
    io::{BufRead, Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};
use usc_abi_encoding::common::EncodingVersion;

const NUMBER: u64 = 1_000_000;

/// The old behaviour backed off for minutes before giving up; anything that has not failed by
/// now is still retrying.
const FAIL_FAST: Duration = Duration::from_secs(5);

type Handler = fn(&str) -> Result<Value, (i64, &'static str)>;

/// HTTP JSON-RPC node answering every request through `handler`.
struct Mock {
    url: String,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Mock {
    fn start(handler: Handler) -> Self {
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
                let reply = match request["method"].as_str().unwrap() {
                    "eth_chainId" => Ok(json!("0x1")),
                    "eth_blockNumber" => Ok(json!(format!("{NUMBER:#x}"))),
                    method => handler(method),
                };
                let out = serde_json::to_vec(&match reply {
                    Ok(result) => json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
                    Err((code, message)) => json!({
                        "jsonrpc":"2.0",
                        "id":request["id"],
                        "error":{"code":code,"message":message}
                    }),
                })
                .unwrap();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", out.len()).unwrap();
                stream.write_all(&out).unwrap();
            }
        });
        Self {
            url,
            stop,
            thread: Some(thread),
        }
    }

    async fn raw_rlp_client(&self) -> Client {
        Client::new(&self.url, None)
            .await
            .unwrap()
            .with_fetch_mode(BlockFetchMode::RawRlp)
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}

/// A node without the `debug` namespace.
fn no_debug_namespace(method: &str) -> Result<Value, (i64, &'static str)> {
    Err((
        -32601,
        if method.starts_with("debug_") {
            "the method does not exist/is not available"
        } else {
            "unexpected method"
        },
    ))
}

/// A node whose raw block is not valid RLP.
fn garbage_raw_block(method: &str) -> Result<Value, (i64, &'static str)> {
    match method {
        "debug_getRawBlock" => Ok(json!("0xdeadbeef")),
        _ => Err((-32601, "unexpected method")),
    }
}

async fn fetch(client: &Client) -> Error {
    match tokio::time::timeout(FAIL_FAST, client.get_block(NUMBER, EncodingVersion::V1)).await {
        Ok(Ok(_)) => panic!("the fetch must fail"),
        Ok(Err(user::Interrupt::Cont(err))) => err,
        Ok(Err(user::Interrupt::Stop)) => panic!("not a shutdown"),
        Err(_) => panic!("the fetch is still retrying after {FAIL_FAST:?}"),
    }
}

#[tokio::test]
async fn missing_debug_namespace_fails_the_fetch_at_once() {
    let mock = Mock::start(no_debug_namespace);
    let err = fetch(&mock.raw_rlp_client().await).await;
    assert!(err.is_unrecoverable(), "{err}");
}

#[tokio::test]
async fn undecodable_raw_block_fails_the_fetch_at_once() {
    let mock = Mock::start(garbage_raw_block);
    let err = fetch(&mock.raw_rlp_client().await).await;
    assert!(matches!(err, Error::RawBlockDecode(NUMBER, _)), "{err}");
}

#[tokio::test]
async fn fetch_mode_check_rejects_a_node_without_the_debug_namespace() {
    let mock = Mock::start(no_debug_namespace);
    let err = mock
        .raw_rlp_client()
        .await
        .check_fetch_mode(FAIL_FAST)
        .await
        .expect_err("raw-RLP needs the debug namespace");
    assert!(
        matches!(
            err,
            Error::FetchModeUnsupported {
                mode: BlockFetchMode::RawRlp,
                ..
            }
        ),
        "{err}"
    );
}

#[tokio::test]
async fn fetch_mode_check_tolerates_transient_failures() {
    // A node that does not have the block yet is not a reason to refuse to start.
    fn not_found(_: &str) -> Result<Value, (i64, &'static str)> {
        Ok(json!("0x"))
    }
    let mock = Mock::start(not_found);
    mock.raw_rlp_client()
        .await
        .check_fetch_mode(FAIL_FAST)
        .await
        .unwrap();
}
