//! `Client::reconnect` against an endpoint that switched chains (a DNS, load-balancer or provider
//! flip): the client must refuse the foreign chain and stay pinned to the one it was built on.
use eth::{Client, Error};
use serde_json::{json, Value};
use std::{
    io::{BufRead, Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

/// Answers `eth_chainId` with whatever `chain_id` currently holds and `eth_blockNumber` with 100.
struct SwitchableChain {
    url: String,
    chain_id: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl SwitchableChain {
    fn start(chain_id: u64) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let id = Arc::new(AtomicU64::new(chain_id));
        let stop = Arc::new(AtomicBool::new(false));
        let (served, done) = (id.clone(), stop.clone());
        let thread = thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(_) => {
                        thread::sleep(Duration::from_millis(2));
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
                let result = match request["method"].as_str() {
                    Some("eth_chainId") => json!(format!("{:#x}", served.load(Ordering::SeqCst))),
                    Some("eth_blockNumber") => json!("0x64"),
                    _ => Value::Null,
                };
                let out = serde_json::to_vec(
                    &json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
                )
                .unwrap();
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    out.len()
                );
                let _ = stream.write_all(&out);
            }
        });
        Self {
            url,
            chain_id: id,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for SwitchableChain {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[tokio::test]
async fn a_reconnect_onto_another_chain_is_refused_and_the_pin_is_kept() {
    let node = SwitchableChain::start(1337);
    let mut client = Client::new(&node.url, None).await.unwrap();
    assert_eq!(client.chain_id(), 1337);

    // Same chain: an ordinary repair.
    client.reconnect().await.unwrap();
    assert_eq!(client.chain_id(), 1337);

    // The endpoint now serves another chain.
    node.chain_id.store(56, Ordering::SeqCst);
    match client.reconnect().await {
        Err(Error::ChainIdChanged { expected, got }) => {
            assert_eq!((expected, got), (1337, 56));
        }
        other => panic!("expected ChainIdChanged, got {other:?}"),
    }
    assert_eq!(client.chain_id(), 1337, "the client stays pinned");

    // Back on the pinned chain, the repair goes through again.
    node.chain_id.store(1337, Ordering::SeqCst);
    client.reconnect().await.unwrap();
    assert_eq!(client.chain_id(), 1337);
}
