//! Startup-time helpers.
//!
//! Each helper is just an `async fn` that returns `Result<…, Error>`. Cancellation is handled
//! by the caller via `tokio::select!` on the cancellation token — these helpers themselves
//! don't sprinkle `ctrl_c` arms everywhere like v1 did.

use std::sync::Arc;

use anyhow::Context as _;
use bls_signatures::Serialize as _;
// `AccountId32` is a newtype over `[u8; 32]`, so this encodes to the account's 32 raw bytes —
// byte-identical to what the runtime's `T::AccountId` encoding feeds the same helper.
use futures::{StreamExt as _, TryStreamExt as _};
use parity_scale_codec::Encode as _;
use tokio_util::sync::CancellationToken;

use attestor_primitives::{AttestorStatus, ChainKey};
use cc_client::{AccountId32, Client};

use crate::error::Error;
use crate::secret::RpcSecret;

/// Loop until both RPCs are reachable. Returns once both answer, or
/// [`Error::ShutdownDuringStartup`] if cancellation fires while we wait.
///
/// The CC3 RPC is always a WebSocket. The Eth RPC may be an HTTP-only proxy (eRPC), which the
/// streams then follow by polling (see `stream::eth::roots`), so its probe is scheme-aware.
pub async fn wait_for_endpoints(
    token: &CancellationToken,
    url_eth: &RpcSecret,
    url_cc3: &RpcSecret,
) -> Result<(), Error> {
    use common::constants::RETRY_DELAY;

    async fn poke(label: &str, url: &RpcSecret, probe: fn(&RpcSecret) -> ProbeFuture<'_>) {
        loop {
            match probe(url).await {
                Ok(()) => return,
                Err(err) => {
                    tracing::info!(%url, %err, "🛜 waiting for {label} rpc...");
                    tokio::time::sleep(RETRY_DELAY).await;
                }
            }
        }
    }

    tokio::select! {
        _ = token.cancelled() => Err(Error::ShutdownDuringStartup),
        () = async {
            poke("Eth", url_eth, |url| Box::pin(probe_eth(url))).await;
            poke("CC3", url_cc3, |url| Box::pin(probe_ws(url))).await;
        } => Ok(()),
    }
}

type ProbeFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>>;

/// One WebSocket handshake.
async fn probe_ws(url: &RpcSecret) -> Result<(), String> {
    tokio_tungstenite::connect_async(url.as_ref())
        .await
        .map(|_| ())
        .map_err(|err| err.to_string())
}

/// Reachability of the Eth RPC: a ws(s) URL must complete a WebSocket handshake; an http(s) URL
/// must answer `eth_chainId`, which is what building an [`eth::Client`] does. Until 3.141 this
/// was the handshake alone, and an http URL looped here forever ("URL scheme not supported").
async fn probe_eth(url: &RpcSecret) -> Result<(), String> {
    if eth::scheme_supports_subscriptions(url.scheme()) {
        probe_ws(url).await
    } else {
        eth::Client::new(url.as_ref().as_str(), None)
            .await
            .map(|_| ())
            .map_err(|err| err.to_string())
    }
}

/// Register a BLS key with the runtime if our status is `Idle`.
pub async fn register_bls(
    chain_key: ChainKey,
    cc3: &Arc<Client>,
    account_id: &AccountId32,
    bls_key: &bls_signatures::PrivateKey,
) -> Result<(), Error> {
    let status = cc3.get_attestor_status(chain_key).await?;
    match status {
        Some(AttestorStatus::Idle) => {}
        // Not in the attestor pool at all. `attest()` cannot be self-served from
        // here — a stash has to call `register_attestor` first — so skipping is
        // correct, but say why rather than reporting the opposite.
        None => {
            tracing::warn!(
                %account_id,
                chain_key,
                "⚠️ not registered as an attestor for this chain — skipping attest(). \
                 A stash account must call register_attestor(chain_key, attestor_id) first, \
                 using an account other than the attestor itself."
            );
            return Ok(());
        }
        Some(other) => {
            tracing::info!(status = ?other, %account_id, "ℹ️ skipping attest() — already registered");
            return Ok(());
        }
    }

    // bls_signatures uses BLS12-381 minimal-pubkey-size: public key is 48 bytes (G1),
    // signature is 96 bytes (G2). The runtime's `start_attesting` extrinsic expects them in
    // that same order (pubkey: [u8; 48], pop: [u8; 96]).
    let public: [u8; 48] = bls_key.public_key().as_bytes()[..]
        .try_into()
        .context("bls public key length")
        .map_err(Error::Init)?;
    // Signed over the runtime's canonical proof-of-possession message, not over the bare public
    // key: the runtime binds the proof to `(chain_key, attestor_id)` so it cannot be replayed by
    // another controller. Built by the shared helper rather than reconstructed here — the two
    // sides drifting means every attestor fails to register.
    let pop_message = attestor_primitives::proof_of_possession_message(
        chain_key,
        account_id.encode().as_slice(),
        &public,
    );
    let pop: [u8; 96] = bls_key.sign(pop_message).as_bytes()[..]
        .try_into()
        .context("bls signature length")
        .map_err(Error::Init)?;

    tracing::info!(%account_id, "📝 Submitting attest() to transition Idle → Waiting");
    cc3.start_attesting(chain_key, public, pop).await?;
    tracing::info!(%account_id, "✅ attest() submitted");
    Ok(())
}

/// Wait until `account_id` is in the active attestor set. Listens to `AttestorsElected`. Returns
/// [`Error::ShutdownDuringStartup`] if cancellation fires while we wait.
pub async fn wait_for_eligible(
    token: &CancellationToken,
    chain_key: ChainKey,
    cc3: &Arc<Client>,
    account_id: &AccountId32,
) -> Result<Vec<AccountId32>, Error> {
    use cc_client::attestation::CcEvent;

    let mut attestors = cc3.get_attestor_active_set(chain_key).await?;
    if attestors.contains(account_id) {
        tracing::info!(%account_id, "☀️ already eligible — warming up before attesting");
        // Same committee warm-up as the freshly-elected path below. "Already in the active
        // set at boot" includes the race where the election committed moments ago (e.g. a
        // restart landing right on an epoch boundary): peers received the same
        // `AttestorsElected` event and may still be refreshing their BLS stores, so gossiping
        // immediately gets our first votes rejected as UnknownAttestor — and gossipsub marks
        // rejected messages seen, so those votes are not redelivered. One warm-up window per
        // boot is a cheaper price than permanently losing the restart-window votes at peers
        // that hadn't refreshed yet.
        tokio::select! {
            _ = token.cancelled() => return Err(Error::ShutdownDuringStartup),
            _ = tokio::time::sleep(common::constants::POST_ELECTION_WARMUP) => {}
        }
        return Ok(attestors);
    }

    let config = stream::cc3::ConfigBuilder::new()
        .with_cc3(Arc::clone(cc3))
        .with_chain_keys(vec![chain_key])
        .build();
    let mut events = stream::cc3::StreamCC3::new(config)
        .await
        .map_err(Error::Init)?;

    let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        tokio::select! {
            _ = token.cancelled() => return Err(Error::ShutdownDuringStartup),
            Some(mut batch) = events.next() => {
                while let Some(event) = batch.try_next().await? {
                    if let CcEvent::AttestorsElected(key, list) = event {
                        if key == chain_key && list.contains(account_id) {
                            attestors = list;
                            tracing::info!(%account_id, "☀️ elected — warming up before attesting");
                            // Give the rest of the committee time to process the same election
                            // (refresh BLS stores, admit our peer) before we start producing —
                            // otherwise our first votes arrive before peers can verify them and
                            // are rejected as UnknownAttestor (feeding their peer-scoring
                            // penalty against us).
                            tokio::select! {
                                _ = token.cancelled() => return Err(Error::ShutdownDuringStartup),
                                _ = tokio::time::sleep(common::constants::POST_ELECTION_WARMUP) => {}
                            }
                            return Ok(attestors);
                        }
                    }
                }
            }
            _ = tick.tick() => {
                // Storage is authoritative. An election finalized between the initial read above
                // and the moment the event stream was seeded is never delivered as an event, and
                // since elections are only emitted when the committee changes there is no later
                // heartbeat to catch it. Re-reading here closes that gap.
                match cc3.get_attestor_active_set(chain_key).await {
                    Ok(list) if list.contains(account_id) => {
                        tracing::info!(%account_id, "☀️ found in the active set — warming up before attesting");
                        tokio::select! {
                            _ = token.cancelled() => return Err(Error::ShutdownDuringStartup),
                            _ = tokio::time::sleep(common::constants::POST_ELECTION_WARMUP) => {}
                        }
                        return Ok(list);
                    }
                    Ok(_) => tracing::info!(%account_id, "⏲️ waiting on election..."),
                    Err(err) => {
                        tracing::warn!(%account_id, error = %err, "⏲️ waiting on election (active-set re-read failed; will retry)")
                    }
                }
            }
        }
    }
}

/// Look up the starting attestation point.
///
/// Returns `(genesis_height, start_attestation)`:
/// - `genesis_height`: the chain's attestation-genesis block (from runtime).
/// - `start_attestation`: the latest finalized anchor — `Some` whether it is backed by a committed
///   attestation *or* by a checkpoint (the two are indistinguishable, and must be, for resume
///   purposes); `None` only if the chain has neither, i.e. we're genuinely starting from genesis.
pub async fn fetch_start_point(
    chain_key: ChainKey,
    cc3: &Client,
) -> Result<
    (
        attestor_primitives::Height,
        Option<crate::shared::AttestationInfo>,
    ),
    Error,
> {
    let genesis = cc3
        .get_attestation_chain_genesis_block_number(chain_key)
        .await?;

    // Resume from the finalized *anchor* — the `(height, digest)` pair the runtime reports —
    // without resolving it back through `Attestations`.
    //
    // The anchor may be backed by either a committed attestation or a checkpoint, and
    // `fetch_last_finalized` already mirrors the runtime's lookup order (`LastDigest`, else
    // `LastCheckpoint`). Resolving it through `Attestations` was wrong for the checkpoint-backed
    // case, which arises two ways:
    //
    //   * after `revert_to()`, which clears every stored attestation for the chain and repoints
    //     *both* `LastCheckpoint` and `LastDigest` at the surviving checkpoint, so `LastDigest`
    //     names a digest that has no `Attestations` entry; and
    //   * on a checkpoint-only chain, where `LastDigest` is absent and the lookup falls back to
    //     `LastCheckpoint` for the same reason.
    //
    // In both cases startup would look up a digest with no attestation entry, treat that valid
    // state as impossible, and fail — deterministically, on every restart, so a supervisor would
    // crash-loop the whole fleet until an operator intervened (ATTESTOR-V2-009).
    //
    // Taking the anchor as-is covers all four states uniformly: attestation-backed,
    // checkpoint-backed after a revert, checkpoint-only, and no anchor at all (true genesis).
    let start = cc3
        .fetch_last_finalized(chain_key)
        .await?
        .map(|(height, digest)| crate::shared::AttestationInfo { height, digest });

    Ok((genesis, start))
}

/// Compare the metadata the binary was compiled against with the live chain metadata.
///
/// If the Attestation pallet hash matches, the live `OnlineClient` already cached the live
/// metadata at construction time, so we just log the comparison and continue. If the
/// Attestation pallet has drifted, refuse to boot — this binary cannot produce valid
/// `commit_attestation` extrinsics for that runtime.
pub async fn reconcile_metadata(cc3: &Arc<Client>) -> Result<(), Error> {
    const ATTESTATION_PALLET: &str = "Attestation";

    let compiled = cc_client::compiled_metadata()
        .map_err(|e| Error::Init(anyhow::anyhow!("decode bundled metadata: {e}")))?;
    let compiled_hash = compiled
        .pallet_by_name(ATTESTATION_PALLET)
        .map(|p| p.hash())
        .ok_or_else(|| {
            Error::Init(anyhow::anyhow!(
                "{ATTESTATION_PALLET} pallet missing from bundled metadata"
            ))
        })?;

    let api = cc3.api();
    let live = api.metadata();
    let live_hash = live
        .pallet_by_name(ATTESTATION_PALLET)
        .map(|p| p.hash())
        .ok_or_else(|| {
            Error::Init(anyhow::anyhow!(
                "{ATTESTATION_PALLET} pallet missing from live chain metadata"
            ))
        })?;

    if compiled_hash != live_hash {
        return Err(Error::Init(anyhow::anyhow!(
            "{ATTESTATION_PALLET} pallet metadata mismatch: \
             compiled={}, live={} — binary needs rebuild against the current chain",
            hex::encode(compiled_hash),
            hex::encode(live_hash),
        )));
    }

    let compiled_full = compiled.hasher().hash();
    let live_full = live.hasher().hash();
    if compiled_full != live_full {
        tracing::info!(
            compiled_full = %hex::encode(compiled_full),
            live_full = %hex::encode(live_full),
            "🧭 chain runtime metadata differs from bundled — \
             Attestation pallet matches, continuing with live metadata"
        );
    } else {
        tracing::info!(
            attestation_hash = %hex::encode(compiled_hash),
            "🧭 chain runtime metadata matches bundled"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal JSON-RPC server that answers every POST with chain id 1.
    async fn http_rpc() -> String {
        use axum::{routing::post, Router};
        let app = Router::new().route(
            "/",
            post(|| async {
                (
                    [("content-type", "application/json")],
                    r#"{"jsonrpc":"2.0","id":1,"result":"0x1"}"#,
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}/")
    }

    fn secret(url: &str) -> RpcSecret {
        RpcSecret::new_exposed(url::Url::parse(url).unwrap())
    }

    #[tokio::test]
    async fn an_http_eth_endpoint_that_answers_eth_chain_id_is_reachable() {
        let url = http_rpc().await;
        assert_eq!(probe_eth(&secret(&url)).await, Ok(()));
    }

    #[tokio::test]
    async fn a_closed_http_port_is_not_reachable() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        assert!(probe_eth(&secret(&format!("http://{addr}/")))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn a_ws_url_on_an_http_only_server_is_not_reachable() {
        // The handshake path is still used for ws(s) schemes: a plain JSON-RPC server rejects it.
        let url = http_rpc().await.replace("http://", "ws://");
        assert!(probe_eth(&secret(&url)).await.is_err());
    }
}
