//! Periodic on-chain attestor-set watcher.
//!
//! Polls `IVoteValidator.attestors()` **and** `IVoteValidator.threshold()` on the destination
//! `EOAValidator` and hot-swaps [`MessageVoteState::active_set`] plus the aggregator threshold
//! whenever either changes. That lets an operator add/remove an attestor **or** retune the quorum
//! on-chain and have every running attestor track it **without a restart** — closing the "set
//! resolved once at startup" gap. Only spawned for
//! [`AttestorSet::OnChainValidator`](super::config::AttestorSet); a static set never changes.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use alloy::primitives::{Address, U256};
use alloy::providers::{Provider, ProviderBuilder};
use tokio_util::sync::CancellationToken;

use super::MessageVoteState;

/// How often to re-read the on-chain attestor set. Changes are rare, so a slow poll keeps RPC load
/// negligible while bounding how long the attestor validates against a stale set.
const ATTESTOR_SET_POLL_SECS: u64 = 30;

/// Resolve the quorum threshold to enforce locally, given the destination validator's own
/// `threshold()` and the current attestor-set size (audit P3-2). We treat the **on-chain**
/// `threshold()` as authoritative — it is exactly what the `EOAValidator` enforces on delivery, so
/// mirroring it keeps the attestor's local "ready for relayer" signal in agreement with the
/// contract even when the operator retunes the threshold without changing membership. A value that
/// is unusable (zero, or larger than the set — a misconfiguration we must not silently under-count
/// against) falls back to the local `2N/3+1` model.
fn effective_threshold(onchain: u64, set_len: usize, validator: Address) -> usize {
    let local = attestor_primitives::calculate_threshold(set_len as u32) as usize;
    let onchain = usize::try_from(onchain).unwrap_or(usize::MAX);
    if onchain >= 1 && onchain <= set_len {
        onchain
    } else {
        tracing::warn!(
            %validator,
            onchain,
            set_len,
            local,
            "on-chain EOAValidator.threshold() is out of range for the current set — using the local 2N/3+1 threshold"
        );
        local
    }
}

/// Watch the destination validator's attestor set + threshold and apply changes to `state` until
/// `token` fires. Best-effort: connection/read failures are logged and retried; the attestor keeps
/// validating against the last-known-good set/threshold in the meantime.
///
/// Connects *per poll* (via [`fetch_attestor_set_and_threshold`]) rather than holding one long-lived
/// provider. A bare alloy WS provider's pubsub service exits permanently after a single failed
/// reconnect, so a held provider would silently stop reading after one routine RPC blip — the set
/// would freeze and, with the post-F3 `Ignore` semantics, votes from newly-added attestors would go
/// uncounted fleet-wide with no recovery short of a restart (S1). A fresh connection each tick is
/// cheap at the 30s cadence and self-heals across blips; a failed tick just retries the next one.
pub async fn watch(
    state: Arc<MessageVoteState>,
    validator: Address,
    dest_rpc_url: String,
    token: CancellationToken,
) {
    tracing::info!(%validator, "🛂 attestor-set watcher online");

    // `interval` fires immediately on the first tick, which is what performs the *initial* population:
    // `build_state` starts the OnChainValidator set empty (to avoid blocking core startup on the
    // destination RPC), so this first tick reads the real set + threshold and swaps them in.
    let mut tick = tokio::time::interval(Duration::from_secs(ATTESTOR_SET_POLL_SECS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // The threshold currently applied to the aggregator. Tracked so a threshold-ONLY change (same
    // membership) is still detected and applied (audit P3-2) — membership equality alone would skip
    // it. `None` until the first successful poll.
    let mut current_threshold: Option<usize> = None;

    loop {
        tokio::select! {
            () = token.cancelled() => {
                tracing::info!("🛑 attestor-set watcher exiting on cancel");
                return;
            }
            _ = tick.tick() => {
                // Fresh connection each poll — see the fn doc. A dead/transiently-unreachable RPC
                // fails only this tick and is retried on the next, instead of wedging the watcher.
                let (set, onchain_threshold) =
                    match fetch_attestor_set_and_threshold(&dest_rpc_url, validator).await {
                        Ok(v) => v,
                        Err(err) => {
                            tracing::warn!(%validator, %err, "failed to read on-chain attestor set/threshold; will retry");
                            continue;
                        }
                    };
                if set.is_empty() {
                    tracing::warn!(%validator, "EOAValidator.attestors() returned empty — keeping current set");
                    continue;
                }

                let threshold = effective_threshold(onchain_threshold, set.len(), validator);
                let membership_changed = *state.active_set.read() != set;
                let threshold_changed = current_threshold != Some(threshold);
                if !membership_changed && !threshold_changed {
                    continue;
                }

                // Swap the set (only if membership actually changed) and re-evaluate the quorum.
                let old_len = {
                    let mut guard = state.active_set.write();
                    let old = guard.len();
                    if membership_changed {
                        *guard = set.clone();
                    }
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
                    if membership_changed {
                        agg.retain_signers(&set, now);
                    }
                    agg.set_threshold(threshold, now)
                };
                for hash in newly_completed {
                    tracing::info!(
                        message_hash = %alloy::primitives::B256::from(hash),
                        threshold,
                        "🎯 message vote reached quorum under the reloaded threshold — ready for relayer delivery"
                    );
                }
                current_threshold = Some(threshold);
                tracing::info!(
                    %validator,
                    old = old_len,
                    new = set.len(),
                    threshold,
                    membership_changed,
                    threshold_changed,
                    "🔄 attestor set/threshold hot-reloaded from EOAValidator"
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

async fn read_threshold<P: Provider>(provider: &P, validator: Address) -> anyhow::Result<U256> {
    let ret = write_ability::abi::IVoteValidator::new(validator, provider)
        .threshold()
        .call()
        .await?;
    Ok(ret._0)
}

/// Connect to the destination chain and read the validator's attestor set **and** quorum threshold
/// once (one connection, two calls). Both are trust-critical and read together so the local view
/// cannot apply a new set against a stale threshold or vice versa.
pub(super) async fn fetch_attestor_set_and_threshold(
    dest_rpc_url: &str,
    validator: Address,
) -> anyhow::Result<(HashSet<Address>, u64)> {
    let provider = ProviderBuilder::new()
        .on_builtin(dest_rpc_url)
        .await
        .map_err(|err| anyhow::anyhow!("connect destination chain RPC: {err}"))?;
    let set = read_attestors(&provider, validator).await?;
    let threshold = read_threshold(&provider, validator).await?;
    Ok((set, u64::try_from(threshold).unwrap_or(u64::MAX)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: Address = Address::ZERO;

    #[test]
    fn effective_threshold_prefers_sane_onchain_value() {
        // On-chain threshold within [1, set_len] is authoritative even when it differs from 2N/3+1.
        assert_eq!(effective_threshold(2, 5, V), 2);
        assert_eq!(effective_threshold(5, 5, V), 5);
        // 2N/3+1 for 5 is 4; an on-chain value of 3 still wins (it is what the contract enforces).
        assert_eq!(effective_threshold(3, 5, V), 3);
    }

    #[test]
    fn effective_threshold_falls_back_when_unusable() {
        let local5 = attestor_primitives::calculate_threshold(5) as usize;
        // Zero and over-size on-chain thresholds are misconfigurations → fall back to local 2N/3+1.
        assert_eq!(effective_threshold(0, 5, V), local5);
        assert_eq!(effective_threshold(6, 5, V), local5);
        assert_eq!(effective_threshold(u64::MAX, 5, V), local5);
    }
}
