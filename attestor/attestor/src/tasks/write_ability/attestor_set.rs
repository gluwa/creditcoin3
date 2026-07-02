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
pub async fn watch(
    state: Arc<MessageVoteState>,
    validator: Address,
    dest_rpc_url: String,
    token: CancellationToken,
) {
    let provider = match ProviderBuilder::new()
        .on_builtin(dest_rpc_url.as_str())
        .await
    {
        Ok(p) => p,
        Err(err) => {
            tracing::error!(
                %validator, %err,
                "attestor-set watcher could not connect to destination RPC — set will not hot-reload"
            );
            return;
        }
    };
    tracing::info!(%validator, "🛂 attestor-set watcher online");

    // `interval` fires immediately on the first tick — but `build_state` already resolved the set at
    // startup, so the first tick typically observes no change.
    let mut tick = tokio::time::interval(Duration::from_secs(ATTESTOR_SET_POLL_SECS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            () = token.cancelled() => {
                tracing::info!("🛑 attestor-set watcher exiting on cancel");
                return;
            }
            _ = tick.tick() => {
                let set = match read_attestors(&provider, validator).await {
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
                state.aggregator.lock().set_threshold(new_threshold);
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
