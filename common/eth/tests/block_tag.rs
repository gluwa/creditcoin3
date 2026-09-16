//! `Client::get_block_number_by_tag` and `Maturity::mature_height` against a tiny in-process
//! JSON-RPC server: tags resolve to header numbers, a `null` tag is "not found", and a primary that
//! errors falls through to the fallback provider.
use eth::{BlockTag, Client, Error, Maturity, TaggedBlock};
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

const ZERO32: &str = "0x0000000000000000000000000000000000000000000000000000000000000000";

/// Header-only block JSON for `number`, enough for alloy's `Block` to deserialize. The hash
/// encodes `salt` and `number`, so two mocks with the same salt agree on every block (one canonical
/// chain) and mocks with different salts are on different forks.
fn header(number: u64, salt: u8) -> Value {
    json!({
        "hash": block_hash(number, salt), "parentHash": ZERO32, "sha3Uncles": ZERO32,
        "miner": "0x0000000000000000000000000000000000000000",
        "stateRoot": ZERO32, "transactionsRoot": ZERO32, "receiptsRoot": ZERO32,
        "logsBloom": format!("0x{}", "00".repeat(256)),
        "difficulty": "0x0", "number": format!("{number:#x}"), "gasLimit": "0x1c9c380",
        "gasUsed": "0x0", "timestamp": "0x64", "extraData": "0x", "mixHash": ZERO32,
        "nonce": "0x0000000000000000", "baseFeePerGas": "0x1",
        "transactions": [], "uncles": []
    })
}

fn block_hash(number: u64, salt: u8) -> String {
    format!("0x{salt:02x}{number:062x}")
}

struct Mock {
    url: String,
    tag_reads: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Mock {
    /// `safe` / `finalized` answer with the given numbers (`None` = JSON `null`); `latest` is 100.
    /// `fail_tags` makes tag lookups return a JSON-RPC error instead.
    fn start(safe: Option<u64>, finalized: Option<u64>, fail_tags: bool) -> Self {
        Self::start_on_fork(safe, finalized, fail_tags, 0)
    }

    /// Answers `eth_chainId` (so the client can be constructed) and then black-holes every other
    /// request: accepts it, reads it, never replies.
    fn start_black_hole() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let thread = thread::spawn(move || {
            let mut held = Vec::new();
            while !done.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(_) => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
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
                if request["method"].as_str() == Some("eth_chainId") {
                    let out = serde_json::to_vec(
                        &json!({"jsonrpc":"2.0","id":request["id"],"result":"0x539"}),
                    )
                    .unwrap();
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", out.len()).unwrap();
                    stream.write_all(&out).unwrap();
                } else {
                    held.push(stream); // never answered
                }
            }
        });
        Self {
            url,
            tag_reads: Arc::new(AtomicUsize::new(0)),
            stop,
            thread: Some(thread),
        }
    }

    /// Like [`Mock::start`], with every block hash salted by `fork` so mocks with different salts
    /// disagree on block identity.
    fn start_on_fork(safe: Option<u64>, finalized: Option<u64>, fail_tags: bool, fork: u8) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let tag_reads = Arc::new(AtomicUsize::new(0));
        let reads = tag_reads.clone();
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
                let response = match request["method"].as_str().unwrap() {
                    "eth_chainId" => json!({"jsonrpc":"2.0","id":request["id"],"result":"0x539"}),
                    "eth_blockNumber" => {
                        json!({"jsonrpc":"2.0","id":request["id"],"result":"0x64"})
                    }
                    "eth_getBlockByNumber" => {
                        let tag = request["params"][0].as_str().unwrap();
                        let numbered =
                            |n: Option<u64>| n.map(|n| header(n, fork)).unwrap_or(Value::Null);
                        let result = match tag {
                            "safe" | "finalized" => {
                                reads.fetch_add(1, Ordering::SeqCst);
                                if fail_tags {
                                    let out = serde_json::to_vec(&json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32601,"message":"tag unsupported"}})).unwrap();
                                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", out.len()).unwrap();
                                    stream.write_all(&out).unwrap();
                                    continue;
                                }
                                if tag == "safe" {
                                    numbered(safe)
                                } else {
                                    numbered(finalized)
                                }
                            }
                            "latest" => header(100, fork),
                            n => header(
                                u64::from_str_radix(n.trim_start_matches("0x"), 16).unwrap(),
                                fork,
                            ),
                        };
                        json!({"jsonrpc":"2.0","id":request["id"],"result":result})
                    }
                    m => panic!("unexpected method {m}"),
                };
                let out = serde_json::to_vec(&response).unwrap();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", out.len()).unwrap();
                stream.write_all(&out).unwrap();
            }
        });
        Self {
            url,
            tag_reads,
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
async fn tags_resolve_to_header_numbers() {
    let mock = Mock::start(Some(90), Some(70), false);
    let client = Client::new(&mock.url, None).await.unwrap();
    assert_eq!(
        client
            .get_block_number_by_tag(BlockTag::Safe)
            .await
            .unwrap(),
        90
    );
    assert_eq!(
        client
            .get_block_number_by_tag(BlockTag::Finalized)
            .await
            .unwrap(),
        70
    );
    assert_eq!(mock.tag_reads.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn null_tag_is_not_found_and_fixed_lag_needs_no_rpc() {
    let mock = Mock::start(None, None, false);
    let client = Client::new(&mock.url, None).await.unwrap();
    assert!(matches!(
        client.get_block_number_by_tag(BlockTag::Safe).await,
        Err(Error::FailedToGetBlockByTag(BlockTag::Safe))
    ));
    assert!(matches!(
        Maturity::Tag(BlockTag::Finalized)
            .mature_height(&client, 100)
            .await,
        Err(Error::FailedToGetBlockByTag(BlockTag::Finalized))
    ));
    let reads_before = mock.tag_reads.load(Ordering::SeqCst);
    assert_eq!(
        Maturity::FixedLag(20)
            .mature_height(&client, 100)
            .await
            .unwrap(),
        Some(80)
    );
    assert_eq!(
        Maturity::FixedLag(200)
            .mature_height(&client, 100)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        mock.tag_reads.load(Ordering::SeqCst),
        reads_before,
        "fixed lag must not touch the RPC"
    );
}

#[tokio::test]
async fn tag_maturity_is_clamped_to_the_observed_head() {
    let mock = Mock::start(Some(95), Some(70), false);
    let client = Client::new(&mock.url, None).await.unwrap();
    // A load-balanced endpoint can report a `safe` block ahead of the head the subscription
    // delivered; maturity never runs ahead of what has been seen.
    assert_eq!(
        Maturity::Tag(BlockTag::Safe)
            .mature_height(&client, 92)
            .await
            .unwrap(),
        Some(92)
    );
    assert_eq!(
        Maturity::Tag(BlockTag::Safe)
            .mature_height(&client, 98)
            .await
            .unwrap(),
        Some(95)
    );
    assert_eq!(
        Maturity::Tag(BlockTag::Finalized)
            .mature_height(&client, 98)
            .await
            .unwrap(),
        Some(70)
    );
}

#[tokio::test]
async fn erroring_primary_falls_through_to_the_fallback_provider() {
    let primary = Mock::start(Some(1), Some(1), true);
    let backup = Mock::start(Some(88), Some(66), false);
    let client = Client::new_with_fallbacks(&primary.url, std::slice::from_ref(&backup.url), None)
        .await
        .unwrap();
    assert_eq!(
        client
            .get_block_number_by_tag(BlockTag::Safe)
            .await
            .unwrap(),
        88
    );
    assert_eq!(
        client
            .get_block_number_by_tag(BlockTag::Finalized)
            .await
            .unwrap(),
        66
    );
    assert!(backup.tag_reads.load(Ordering::SeqCst) >= 2);
}

/// The primary's answer leads. A fallback that is further along confirms the candidate's identity
/// at that height; a fallback that is behind is logged and skipped, and cannot drag the boundary
/// down. Both providers are asked the tag each time.
#[tokio::test]
async fn the_primary_leads_and_fallbacks_confirm_or_step_aside() {
    // Fallback ahead of the primary: it confirms the primary's block by hash.
    let primary = Mock::start(Some(88), Some(66), false);
    let ahead = Mock::start(Some(90), Some(70), false);
    let client = Client::new_with_fallbacks(&primary.url, std::slice::from_ref(&ahead.url), None)
        .await
        .unwrap();
    assert_eq!(
        client.get_block_by_tag(BlockTag::Safe).await.unwrap(),
        TaggedBlock {
            number: 88,
            hash: block_hash(88, 0).parse().unwrap(),
        }
    );
    assert_eq!(primary.tag_reads.load(Ordering::SeqCst), 1);
    assert_eq!(ahead.tag_reads.load(Ordering::SeqCst), 1);

    // Fallback behind the primary: it cannot confirm, and it does not hold the primary back.
    let primary = Mock::start(Some(90), Some(70), false);
    let behind = Mock::start(Some(88), Some(66), false);
    let client = Client::new_with_fallbacks(&primary.url, std::slice::from_ref(&behind.url), None)
        .await
        .unwrap();
    assert_eq!(
        client
            .get_block_number_by_tag(BlockTag::Finalized)
            .await
            .unwrap(),
        70
    );
}

/// A fallback that stopped syncing keeps answering the same tag height forever, and its block at
/// that height hashes identically to the canonical chain. It must not pin the boundary.
#[tokio::test]
async fn a_stale_fallback_cannot_pin_maturity() {
    let primary = Mock::start(Some(90), Some(70), false);
    let stale = Mock::start(Some(40), Some(20), false);
    let client = Client::new_with_fallbacks(&primary.url, std::slice::from_ref(&stale.url), None)
        .await
        .unwrap();
    assert_eq!(
        client
            .get_block_number_by_tag(BlockTag::Safe)
            .await
            .unwrap(),
        90,
        "the stale fallback must be skipped, not followed"
    );
    // Without a primary answer the freshest fallback leads, for the same reason.
    let dead_primary = Mock::start(None, None, false);
    let fresh = Mock::start(Some(95), Some(75), false);
    let client = Client::new_with_fallbacks(
        &dead_primary.url,
        &[stale.url.clone(), fresh.url.clone()],
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        client
            .get_block_number_by_tag(BlockTag::Safe)
            .await
            .unwrap(),
        95
    );
}

/// A provider that accepts the connection and never answers must not hang the lookup, and with it
/// the tip stream that awaits the lookup inline.
#[tokio::test]
async fn a_black_holed_provider_times_out_instead_of_hanging_the_lookup() {
    let primary = Mock::start(Some(90), Some(70), false);
    let black_hole = Mock::start_black_hole();
    let client =
        Client::new_with_fallbacks(&primary.url, std::slice::from_ref(&black_hole.url), None)
            .await
            .unwrap()
            .with_call_timeout(Duration::from_millis(500));
    let started = std::time::Instant::now();
    assert_eq!(
        client
            .get_block_number_by_tag(BlockTag::Safe)
            .await
            .unwrap(),
        90
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "lookup took {:?}; the black hole was awaited past its timeout",
        started.elapsed()
    );
}

/// A provider on another fork does not get a vote and does not lose one either: the lookup
/// refuses to pick a side. The caller retries on the next head.
#[tokio::test]
async fn a_forked_provider_is_a_disagreement_not_a_fallback() {
    // Different heights: the further provider is asked for the candidate height and has a
    // different block there.
    let canonical = Mock::start_on_fork(Some(88), Some(66), false, 0);
    let forked = Mock::start_on_fork(Some(90), Some(70), false, 7);
    let client =
        Client::new_with_fallbacks(&canonical.url, std::slice::from_ref(&forked.url), None)
            .await
            .unwrap();
    match client.get_block_by_tag(BlockTag::Safe).await {
        Err(Error::BlockTagDisagreement {
            tag,
            number,
            expected,
            actual,
            ..
        }) => {
            assert_eq!((tag, number), (BlockTag::Safe, 88));
            assert_eq!(format!("{expected:?}"), block_hash(88, 0));
            assert_eq!(format!("{actual:?}"), block_hash(88, 7));
        }
        other => panic!("expected a disagreement, got {other:?}"),
    }

    // Same height, different hash: no extra read needed, still a disagreement.
    let forked_same_height = Mock::start_on_fork(Some(88), Some(66), false, 7);
    let client = Client::new_with_fallbacks(
        &canonical.url,
        std::slice::from_ref(&forked_same_height.url),
        None,
    )
    .await
    .unwrap();
    assert!(matches!(
        client.get_block_by_tag(BlockTag::Finalized).await,
        Err(Error::BlockTagDisagreement { number: 66, .. })
    ));
}

/// The disagreement surfaces through `Maturity` unchanged, so the streams log and retry on the
/// next head rather than treating it as a dead connection.
#[tokio::test]
async fn maturity_surfaces_a_disagreement_as_an_error_for_this_head() {
    let canonical = Mock::start_on_fork(Some(88), Some(66), false, 0);
    let forked = Mock::start_on_fork(Some(88), Some(66), false, 3);
    let client =
        Client::new_with_fallbacks(&canonical.url, std::slice::from_ref(&forked.url), None)
            .await
            .unwrap();
    assert!(matches!(
        Maturity::Tag(BlockTag::Safe)
            .mature_height(&client, 100)
            .await,
        Err(Error::BlockTagDisagreement { .. })
    ));
}
