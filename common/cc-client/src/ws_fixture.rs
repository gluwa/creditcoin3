//! A disposable in-process Creditcoin node speaking just enough Substrate JSON-RPC over
//! WebSocket for [`crate::Client`] to connect, subscribe to finalized heads and read events.
//!
//! Built for liveness tests: the test controls the finalized height, can silence the
//! subscriptions on currently open connections (a subscription that died upstream while the
//! socket stays open), close every open socket (a peer that went away), or start returning
//! pruned-state errors for event reads. No live chain is involved.
//!
//! Chain model: block `h` has hash `0x…h` (the height, hex, zero-padded to 32 bytes) and parent
//! `0x…(h-1)`, so any header can be reconstructed from its hash. Event storage is always empty.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::task::{JoinHandle, JoinSet};
use tokio_tungstenite::tungstenite::Message;

/// Hash of the block at `height`.
#[must_use]
pub fn block_hash(height: u64) -> String {
    format!("0x{height:064x}")
}

/// Inverse of [`block_hash`].
#[must_use]
pub fn height_of(hash: &str) -> u64 {
    u64::from_str_radix(hash.trim_start_matches("0x"), 16).expect("fixture hashes are heights")
}

/// Substrate header JSON for the block at `height`.
#[must_use]
pub fn header(height: u64) -> Value {
    json!({
        "number": format!("0x{height:x}"),
        "parentHash": block_hash(height.saturating_sub(1)),
        "stateRoot": block_hash(0),
        "extrinsicsRoot": block_hash(0),
        "digest": {"logs": []},
    })
}

#[derive(Clone, Copy, Debug)]
enum Signal {
    /// A new finalized head; open, non-silenced subscriptions push it.
    Head(u64),
    /// Every currently open connection stops pushing heads (RPC keeps answering).
    Silence,
}

/// Shared chain state, one per fixture.
#[derive(Debug, Default)]
pub struct FixtureChain {
    finalized: AtomicU64,
    /// After this many `state_getStorage` calls (0 = never) every further one fails with
    /// `State already discarded`, the permanently-pruned path.
    prune_events_after: AtomicUsize,
    event_fetches: AtomicUsize,
    /// Set by the connection tasks; lets tests wait until a subscription exists.
    subscriptions: AtomicUsize,
}

impl FixtureChain {
    #[must_use]
    pub fn finalized(&self) -> u64 {
        self.finalized.load(Ordering::SeqCst)
    }

    /// Start failing event reads with a pruned-state error after `n` successful ones.
    pub fn prune_events_after(&self, n: usize) {
        self.prune_events_after.store(n, Ordering::SeqCst);
    }

    /// Number of `chain_subscribeFinalizedHeads` accepted so far.
    #[must_use]
    pub fn subscriptions(&self) -> usize {
        self.subscriptions.load(Ordering::SeqCst)
    }

    fn storage_read(&self) -> Result<Value, Value> {
        let n = self.event_fetches.fetch_add(1, Ordering::SeqCst) + 1;
        let prune_after = self.prune_events_after.load(Ordering::SeqCst);
        if prune_after > 0 && n > prune_after {
            Err(json!({"code": -32000, "message": "UnknownBlock: State already discarded"}))
        } else {
            Ok(json!("0x00"))
        }
    }
}

pub struct WsFixture {
    pub url: String,
    /// Send to close every currently open connection (the server keeps accepting).
    pub close: broadcast::Sender<()>,
    /// Connections accepted so far.
    pub accepted: Arc<AtomicUsize>,
    pub chain: Arc<FixtureChain>,
    signals: broadcast::Sender<Signal>,
    task: JoinHandle<()>,
}

impl Drop for WsFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl WsFixture {
    /// Start a node whose finalized head is 0 and whose subscriptions push every head the
    /// test finalizes.
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (close, _) = broadcast::channel(16);
        let (signals, _) = broadcast::channel::<Signal>(256);
        let accepted = Arc::new(AtomicUsize::new(0));
        let chain = Arc::new(FixtureChain::default());

        let task = {
            let close = close.clone();
            let signals = signals.clone();
            let accepted = accepted.clone();
            let chain = chain.clone();
            tokio::spawn(async move {
                let mut connections = JoinSet::new();
                loop {
                    let (socket, _) = listener.accept().await.unwrap();
                    accepted.fetch_add(1, Ordering::SeqCst);
                    let close = close.subscribe();
                    let signals = signals.subscribe();
                    let chain = chain.clone();
                    connections.spawn(serve_connection(socket, close, signals, chain));
                }
            })
        };

        Self {
            url,
            close,
            accepted,
            chain,
            signals,
            task,
        }
    }

    /// Advance the finalized head to `height` and notify open, non-silenced subscriptions.
    pub fn finalize(&self, height: u64) {
        self.chain.finalized.store(height, Ordering::SeqCst);
        let _ = self.signals.send(Signal::Head(height));
    }

    /// Subscriptions on every currently open connection go quiet from now on; RPC keeps
    /// answering with the true finalized head. New connections are unaffected.
    pub fn silence_open_subscriptions(&self) {
        let _ = self.signals.send(Signal::Silence);
    }

    /// Close every open connection; the node keeps accepting new ones.
    pub fn close_open_connections(&self) {
        let _ = self.close.send(());
    }
}

async fn serve_connection(
    socket: tokio::net::TcpStream,
    mut close: broadcast::Receiver<()>,
    mut signals: broadcast::Receiver<Signal>,
    chain: Arc<FixtureChain>,
) {
    let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
    let mut subscription: Option<String> = None;
    let mut silent = false;
    loop {
        tokio::select! {
            _ = close.recv() => {
                let _ = ws.close(None).await;
                break;
            }
            signal = signals.recv() => match signal {
                Ok(Signal::Silence) => silent = true,
                Ok(Signal::Head(h)) => {
                    if let (Some(id), false) = (&subscription, silent) {
                        if push_head(&mut ws, id, h).await.is_err() { break; }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            },
            incoming = ws.next() => {
                let text = match incoming {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = ws.send(Message::Pong(payload)).await;
                        continue;
                    }
                    Some(Ok(Message::Pong(_))) => continue,
                    _ => break,
                };
                let req: Value = serde_json::from_str(text.as_str()).unwrap();
                let params = req["params"].as_array().cloned().unwrap_or_default();
                let method = req["method"].as_str().unwrap_or_default().to_owned();
                let finalized = chain.finalized();
                let result: Result<Value, Value> = match method.as_str() {
                    "chain_getFinalizedHead" => Ok(json!(block_hash(finalized))),
                    "chain_getBlockHash" => Ok(json!(block_hash(param_height(params.first(), finalized)))),
                    "chain_getHeader" => Ok(header(
                        params.first().and_then(Value::as_str).map_or(finalized, height_of),
                    )),
                    "state_getStorage" => chain.storage_read(),
                    "state_getRuntimeVersion" => Ok(json!({"specVersion": 1, "transactionVersion": 1})),
                    "state_call" => {
                        let metadata = include_bytes!("../artifacts/metadata.scale");
                        // SCALE Option<OpaqueMetadata>: Some + compact Vec length + bytes.
                        let len = u32::try_from(metadata.len()).expect("metadata fits in u32");
                        let mut encoded = vec![1u8];
                        encoded.extend_from_slice(&((len << 2) | 2).to_le_bytes());
                        encoded.extend_from_slice(metadata);
                        Ok(json!(format!("0x{}", hex::encode(encoded))))
                    }
                    "chain_subscribeFinalizedHeads" => {
                        let id = format!("fixture-sub-{}", chain.subscriptions.fetch_add(1, Ordering::SeqCst) + 1);
                        subscription = Some(id.clone());
                        Ok(json!(id))
                    }
                    "chain_unsubscribeFinalizedHeads" => {
                        subscription = None;
                        Ok(json!(true))
                    }
                    other => panic!("unexpected fixture RPC: {other}"),
                };
                let response = match result {
                    Ok(result) => json!({"jsonrpc": "2.0", "id": req["id"], "result": result}),
                    Err(error) => json!({"jsonrpc": "2.0", "id": req["id"], "error": error}),
                };
                if ws.send(Message::Text(response.to_string().into())).await.is_err() {
                    break;
                }
                // A node catching a new subscriber up: push every head so far, in order.
                if method == "chain_subscribeFinalizedHeads" && !silent {
                    if let Some(id) = &subscription {
                        for h in 1..=finalized {
                            if push_head(&mut ws, id, h).await.is_err() { break; }
                        }
                    }
                }
            }
        }
    }
}

fn param_height(param: Option<&Value>, finalized: u64) -> u64 {
    match param {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(finalized),
        Some(Value::String(s)) => height_of(s),
        _ => finalized,
    }
}

async fn push_head<S>(ws: &mut S, subscription: &str, height: u64) -> Result<(), ()>
where
    S: futures::Sink<Message> + Unpin,
{
    let event = json!({
        "jsonrpc": "2.0",
        "method": "chain_finalizedHead",
        "params": {"subscription": subscription, "result": header(height)},
    });
    ws.send(Message::Text(event.to_string().into()))
        .await
        .map_err(|_| ())
}
