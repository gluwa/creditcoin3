//! Startup-time helpers.
//!
//! Each helper is just an `async fn` that returns `Result<…, Error>`. Cancellation is handled
//! by the caller via `tokio::select!` on the cancellation token — these helpers themselves
//! don't sprinkle `ctrl_c` arms everywhere like v1 did.

use std::sync::Arc;

use anyhow::Context as _;
use async_trait::async_trait;
use bls_signatures::Serialize as _;
use futures::{StreamExt as _, TryStreamExt as _};
use tokio_util::sync::CancellationToken;

use attestor_primitives::{AttestorStatus, ChainKey};
use cc_client::{AccountId32, Client};

use crate::error::Error;
use crate::secret::RpcSecret;

/// Loop until both RPCs accept a WebSocket connection. Returns once both are reachable, or
/// [`Error::ShutdownDuringStartup`] if cancellation fires while we wait.
pub async fn wait_for_endpoints(
    token: &CancellationToken,
    url_eth: &RpcSecret,
    url_cc3: &RpcSecret,
) -> Result<(), Error> {
    use common::constants::RETRY_DELAY;

    async fn poke(label: &str, url: &RpcSecret) {
        loop {
            match tokio_tungstenite::connect_async(url.as_ref()).await {
                Ok(_) => return,
                Err(err) => {
                    tracing::info!(%url, %err, "🛜 waiting for {label} ws...");
                    tokio::time::sleep(RETRY_DELAY).await;
                }
            }
        }
    }

    tokio::select! {
        _ = token.cancelled() => Err(Error::ShutdownDuringStartup),
        () = async {
            poke("Eth", url_eth).await;
            poke("CC3", url_cc3).await;
        } => Ok(()),
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
    if status != Some(AttestorStatus::Idle) {
        tracing::info!(?status, %account_id, "ℹ️ skipping attest() — already registered");
        return Ok(());
    }

    // bls_signatures uses BLS12-381 minimal-pubkey-size: public key is 48 bytes (G1),
    // signature is 96 bytes (G2). The runtime's `start_attesting` extrinsic expects them in
    // that same order (pubkey: [u8; 48], pop: [u8; 96]).
    let public: [u8; 48] = bls_key.public_key().as_bytes()[..]
        .try_into()
        .context("bls public key length")
        .map_err(Error::Init)?;
    let pop: [u8; 96] = bls_key.sign(public).as_bytes()[..]
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
                tracing::info!(%account_id, "⏲️ waiting on election...");
            }
        }
    }
}

/// The chain queries [`fetch_start_point`] needs, abstracted so the resume logic is testable.
///
/// [`Client`] is a concrete wrapper around a live subxt connection, so code taking it directly
/// can only be exercised against a running node — which is why this module had no tests. Depending
/// on this narrow trait instead lets the resume rules (including the checkpoint-backed anchor case
/// that used to crash-loop startup) be covered by ordinary unit tests.
///
/// Method names mirror [`Client`]'s so the production impl is a straight delegation.
#[async_trait]
pub trait StartPointQuery {
    /// First block of the chain's attestation range.
    async fn get_attestation_chain_genesis_block_number(
        &self,
        chain_key: ChainKey,
    ) -> Result<attestor_primitives::Height, Error>;

    /// Latest finalized anchor as `(height, digest)`, following the runtime's lookup order
    /// (`LastDigest`, else `LastCheckpoint`). `None` when the chain has neither.
    async fn fetch_last_finalized(
        &self,
        chain_key: ChainKey,
    ) -> Result<Option<(attestor_primitives::Height, attestor_primitives::Digest)>, Error>;
}

#[async_trait]
impl StartPointQuery for Client {
    async fn get_attestation_chain_genesis_block_number(
        &self,
        chain_key: ChainKey,
    ) -> Result<attestor_primitives::Height, Error> {
        Ok(Client::get_attestation_chain_genesis_block_number(self, chain_key).await?)
    }

    async fn fetch_last_finalized(
        &self,
        chain_key: ChainKey,
    ) -> Result<Option<(attestor_primitives::Height, attestor_primitives::Digest)>, Error> {
        Ok(Client::fetch_last_finalized(self, chain_key).await?)
    }
}

/// Look up the starting attestation point.
///
/// Returns `(genesis_height, start_attestation)`:
/// - `genesis_height`: the chain's attestation-genesis block (from runtime).
/// - `start_attestation`: the latest finalized anchor — `Some` whether it is backed by a committed
///   attestation *or* by a checkpoint (the two are indistinguishable, and must be, for resume
///   purposes); `None` only if the chain has neither, i.e. we're genuinely starting from genesis.
pub async fn fetch_start_point<C: StartPointQuery + ?Sized>(
    chain_key: ChainKey,
    cc3: &C,
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
    use attestor_primitives::{Digest, Height};

    const CHAIN: ChainKey = 7;
    const GENESIS: Height = 1_000;

    /// In-memory [`StartPointQuery`], standing in for a live chain.
    struct FakeChain {
        genesis: Height,
        anchor: Option<(Height, Digest)>,
        fail_anchor: bool,
    }

    impl FakeChain {
        /// Chain with a finalized anchor. Deliberately says nothing about whether the anchor is
        /// backed by an attestation or a checkpoint — startup must not care.
        fn with_anchor(height: Height, digest: Digest) -> Self {
            Self {
                genesis: GENESIS,
                anchor: Some((height, digest)),
                fail_anchor: false,
            }
        }

        /// Chain with no finalized anchor at all (true genesis).
        fn empty() -> Self {
            Self {
                genesis: GENESIS,
                anchor: None,
                fail_anchor: false,
            }
        }

        fn failing() -> Self {
            Self {
                genesis: GENESIS,
                anchor: None,
                fail_anchor: true,
            }
        }
    }

    #[async_trait]
    impl StartPointQuery for FakeChain {
        async fn get_attestation_chain_genesis_block_number(
            &self,
            _chain_key: ChainKey,
        ) -> Result<Height, Error> {
            Ok(self.genesis)
        }

        async fn fetch_last_finalized(
            &self,
            _chain_key: ChainKey,
        ) -> Result<Option<(Height, Digest)>, Error> {
            if self.fail_anchor {
                return Err(Error::MissingMaxCatchup(CHAIN));
            }
            Ok(self.anchor)
        }
    }

    #[tokio::test]
    async fn resumes_from_an_attestation_backed_anchor() {
        let digest = Digest::repeat_byte(0xab);
        let (genesis, start) = fetch_start_point(CHAIN, &FakeChain::with_anchor(1_500, digest))
            .await
            .expect("resume from attestation-backed anchor");

        assert_eq!(genesis, GENESIS);
        assert_eq!(
            start,
            Some(crate::shared::AttestationInfo {
                height: 1_500,
                digest
            })
        );
    }

    /// Regression test for ATTESTOR-V2-009.
    ///
    /// After `revert_to()` the runtime clears every stored attestation and repoints the finalized
    /// anchor at the surviving checkpoint, so the anchor's digest has no `Attestations` entry.
    /// Startup used to resolve the anchor through `Attestations` and therefore rejected this valid
    /// state, failing deterministically on every boot — a supervisor then crash-looped the whole
    /// fleet after an emergency revert. Resuming must succeed regardless of what backs the anchor.
    #[tokio::test]
    async fn resumes_from_a_checkpoint_backed_anchor_after_revert() {
        // The checkpoint digest: valid anchor, no attestation behind it.
        let checkpoint_digest = Digest::repeat_byte(0xcd);
        let (genesis, start) =
            fetch_start_point(CHAIN, &FakeChain::with_anchor(1_200, checkpoint_digest))
                .await
                .expect("checkpoint-backed anchor must not fail startup");

        assert_eq!(genesis, GENESIS);
        assert_eq!(
            start,
            Some(crate::shared::AttestationInfo {
                height: 1_200,
                digest: checkpoint_digest
            })
        );
    }

    #[tokio::test]
    async fn reports_no_anchor_when_chain_is_at_genesis() {
        let (genesis, start) = fetch_start_point(CHAIN, &FakeChain::empty())
            .await
            .expect("genesis chain resolves");

        assert_eq!(genesis, GENESIS);
        assert_eq!(start, None, "no anchor means resume from genesis");
    }

    #[tokio::test]
    async fn propagates_query_errors() {
        let err = fetch_start_point(CHAIN, &FakeChain::failing())
            .await
            .expect_err("anchor query failure must surface");
        assert!(matches!(err, Error::MissingMaxCatchup(CHAIN)));
    }
}
