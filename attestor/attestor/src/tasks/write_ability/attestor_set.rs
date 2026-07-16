//! Periodic on-chain attestor-set watcher.
//!
//! Polls `IVoteValidator.attestors()` on the destination `EOAValidator` and hot-swaps
//! [`MessageVoteState::active_set`] (plus the aggregator threshold) whenever it changes. That lets
//! an operator add/remove an attestor on-chain and have every running attestor accept/reject the
//! corresponding gossip votes **without a restart** — closing the "set resolved once at startup"
//! gap. Only spawned for [`AttestorSet::OnChainValidator`](super::config::AttestorSet); a static set
//! never changes.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use alloy::primitives::Address;
use alloy::providers::{Provider, ProviderBuilder};
use tokio_util::sync::CancellationToken;

use super::MessageVoteState;

/// How often to re-read the on-chain attestor set. Changes are rare, so a slow poll keeps RPC load
/// negligible while bounding how long the attestor validates against a stale set.
const ATTESTOR_SET_POLL_SECS: u64 = 30;

/// Watch the destination validator's attestor set and apply changes to `state` until `token` fires.
/// Best-effort: connection/read failures are logged and retried; the attestor keeps validating
/// against the last-known-good set in the meantime.
///
/// Connects *per poll* (via [`fetch_attestor_set`]) rather than holding one long-lived provider. A
/// bare alloy WS provider's pubsub service exits permanently after a single failed reconnect, so a
/// held provider would silently stop reading after one routine RPC blip — the set would freeze and,
/// with the post-F3 `Ignore` semantics, votes from newly-added attestors would go uncounted
/// fleet-wide with no recovery short of a restart (S1). A fresh connection each tick is cheap at the
/// 30s cadence and self-heals across blips; a failed tick just retries the next one.
pub async fn watch(
    state: Arc<MessageVoteState>,
    validator: Address,
    dest_rpc_url: String,
    token: CancellationToken,
) {
    tracing::info!(%validator, "🛂 attestor-set watcher online");

    // `interval` fires immediately on the first tick, which is what performs the *initial* population:
    // `build_state` starts the OnChainValidator set empty (to avoid blocking core startup on the
    // destination RPC), so this first tick reads the real set and swaps it in.
    let mut tick = tokio::time::interval(Duration::from_secs(ATTESTOR_SET_POLL_SECS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            () = token.cancelled() => {
                tracing::info!("🛑 attestor-set watcher exiting on cancel");
                return;
            }
            _ = tick.tick() => {
                // Fresh connection each poll — see the fn doc. A dead/transiently-unreachable RPC
                // fails only this tick and is retried on the next, instead of wedging the watcher.
                let set = match fetch_attestor_set(&dest_rpc_url, validator).await {
                    Ok(s) => s,
                    Err(err) => {
                        tracing::warn!(%validator, %err, "failed to read on-chain attestor set; will retry");
                        continue;
                    }
                };
                if set.is_empty() {
                    tracing::warn!(%validator, "EOAValidator.attestors() returned empty — keeping current set");
                    continue;
                }
                if *state.active_set.read() == set {
                    continue;
                }

                // Swap the set, then update the quorum threshold to match the new size.
                let new_threshold =
                    attestor_primitives::calculate_threshold(set.len() as u32) as usize;
                let old_len = {
                    let mut guard = state.active_set.write();
                    let old = guard.len();
                    *guard = set.clone();
                    old
                };
                // Prune signatures from signers that just left the set BEFORE re-evaluating the
                // quorum: stale signatures must not keep counting toward completion, or a lowered
                // threshold could mark a message complete with fewer *current* attestors than the
                // destination validator will accept. Then re-evaluate every tracked aggregate
                // against the new quorum; a lowered threshold can push already-collected messages
                // over it, in which case the milestone must be surfaced here — there is no later
                // vote transition to fire it.
                let newly_completed = {
                    let now = std::time::Instant::now();
                    let mut agg = state.aggregator.lock();
                    agg.retain_signers(&set, now);
                    agg.set_threshold(new_threshold, now)
                };
                for hash in newly_completed {
                    tracing::info!(
                        message_hash = %alloy::primitives::B256::from(hash),
                        threshold = new_threshold,
                        "🎯 message vote reached 2/3+1 under the reloaded threshold — ready for relayer delivery"
                    );
                }
                tracing::info!(
                    %validator,
                    old = old_len,
                    new = set.len(),
                    threshold = new_threshold,
                    "🔄 attestor set hot-reloaded from EOAValidator"
                );
            }
        }
    }
}

async fn read_attestors<P: Provider>(
    provider: &P,
    validator: Address,
) -> anyhow::Result<HashSet<Address>> {
    let ret = write_ability::abi::IVoteValidator::new(validator, provider)
        .attestors()
        .call()
        .await?;
    Ok(ret._0.into_iter().collect())
}

/// Connect to the destination chain and read the validator's attestor set once. Shared by the
/// startup resolution in [`super::build_state`] and this module's hot-reload watcher so the
/// trust-critical fetch cannot drift between the two.
pub(super) async fn fetch_attestor_set(
    dest_rpc_url: &str,
    validator: Address,
) -> anyhow::Result<HashSet<Address>> {
    let provider = ProviderBuilder::new()
        .on_builtin(dest_rpc_url)
        .await
        .map_err(|err| anyhow::anyhow!("connect destination chain RPC: {err}"))?;
    read_attestors(&provider, validator).await
}
