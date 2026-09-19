//! A disposable loopback Creditcoin WebSocket node for liveness tests.
//!
//! Answers just enough of the Substrate RPC surface for `cc_client::Client` to connect
//! (metadata, runtime version, finalized head, storage reads) and lets a test close every
//! open socket or serve pruned history on demand. No live chain is involved.

#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::task::{JoinHandle, JoinSet};
use tokio_tungstenite::tungstenite::Message;

pub struct WsFixture {
    pub url: String,
    /// Send to close every currently open connection (the server keeps accepting).
    pub close: broadcast::Sender<()>,
    /// Number of connections accepted so far.
    pub accepted: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl Drop for WsFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub fn mock_header(height: u64) -> Value {
    let hash = format!("0x{}", "11".repeat(32));
    json!({"number": format!("0x{height:x}"), "parentHash": hash, "stateRoot": hash,
        "extrinsicsRoot": hash, "digest": {"logs": []}})
}

impl WsFixture {
    /// Healthy node: answers RPC, emits no finalized-head notifications.
    pub async fn start() -> Self {
        Self::start_with_pruned_events(false).await
    }

    /// When `prune_second_block` is set, the node pushes finalized heads 1 and 2 right after
    /// the subscription is accepted and answers every `state_getStorage` after the first with
    /// `State already discarded`, which is the permanent-pruned path in `StreamCC3`.
    pub async fn start_with_pruned_events(prune_second_block: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (close, _) = broadcast::channel(4);
        let signal = close.clone();
        let accepted = Arc::new(AtomicUsize::new(0));
        let count = accepted.clone();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                count.fetch_add(1, Ordering::SeqCst);
                let mut close = signal.subscribe();
                connections.spawn(async move {
                    let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
                    let mut event_fetches = 0;
                    loop {
                        tokio::select! {
                            _ = close.recv() => {
                                let _ = ws.close(None).await;
                                break;
                            }
                            incoming = ws.next() => {
                                let text = match incoming {
                                    Some(Ok(Message::Text(text))) => text,
                                    Some(Ok(Message::Ping(payload))) => {
                                        let _ = ws.send(Message::Pong(payload)).await;
                                        continue;
                                    }
                                    _ => break,
                                };
                                let req: Value = serde_json::from_str(text.as_str()).unwrap();
                                if req["method"] == "state_getStorage" {
                                    event_fetches += 1;
                                    if prune_second_block && event_fetches > 1 {
                                        let response = json!({"jsonrpc": "2.0", "id": req["id"],
                                            "error": {"code": -32000, "message": "UnknownBlock: State already discarded"}});
                                        let _ = ws.send(Message::Text(response.to_string().into())).await;
                                        continue;
                                    }
                                }
                                let result = match req["method"].as_str().unwrap() {
                                    "chain_getFinalizedHead" | "chain_getBlockHash" => json!(format!("0x{}", "11".repeat(32))),
                                    "chain_getHeader" => mock_header(0),
                                    "state_getStorage" => json!("0x00"),
                                    "state_getRuntimeVersion" => json!({"specVersion": 1, "transactionVersion": 1}),
                                    "state_call" => {
                                        let metadata = include_bytes!("../../../common/cc-client/artifacts/metadata.scale");
                                        // SCALE Option<OpaqueMetadata>: Some + compact Vec length + bytes.
                                        let mut encoded = vec![1u8];
                                        encoded.extend_from_slice(&(((metadata.len() as u32) << 2) | 2).to_le_bytes());
                                        encoded.extend_from_slice(metadata);
                                        json!(format!("0x{}", hex::encode(encoded)))
                                    }
                                    "chain_subscribeFinalizedHeads" => json!("fixture-subscription"),
                                    "chain_unsubscribeFinalizedHeads" => json!(true),
                                    other => panic!("unexpected fixture RPC: {other}"),
                                };
                                let response = json!({"jsonrpc": "2.0", "id": req["id"], "result": result});
                                if ws.send(Message::Text(response.to_string().into())).await.is_err() { break; }
                                if prune_second_block && req["method"] == "chain_subscribeFinalizedHeads" {
                                    for height in [1, 2] {
                                        let event = json!({"jsonrpc": "2.0", "method": "chain_finalizedHead",
                                            "params": {"subscription": "fixture-subscription", "result": mock_header(height)}});
                                        let _ = ws.send(Message::Text(event.to_string().into())).await;
                                    }
                                }
                            }
                        }
                    }
                });
            }
        });
        Self {
            url,
            close,
            accepted,
            task,
        }
    }
}
