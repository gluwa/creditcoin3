//! Periodic on-chain attestor-set watcher.
//!
//! Polls `IVoteValidator.attestors()` **and** `IVoteValidator.threshold()` on the destination
//! `EOAValidator` and hot-swaps [`MessageVoteState::active_set`] plus the aggregator threshold
//! whenever either changes. That lets an operator add/remove an attestor **or** retune the quorum
//! on-chain and have every running attestor track it **without a restart** — closing the "set
//! resolved once at startup" gap. The validator is the attestor's only source of the set.
//!
//! The same poll resolves the message-vote EIP-712 domain ([`MessageVoteState::vote_domain`]):
//! the validator's ERC-5267 `eip712Domain()` must match the name/version this build signs with
//! and name the validator itself as `verifyingContract`, and the configured Inbox must hand its
//! votes to this validator (`defaultVoteValidator()`). Any mismatch clears the domain, which stops
//! signing instead of producing votes the destination would reject.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use alloy::primitives::{Address, U256};
use alloy::providers::{Provider, ProviderBuilder};
use tokio_util::sync::CancellationToken;
use write_ability::hash::{
    MessageVoteDomain, MESSAGE_VOTE_DOMAIN_NAME, MESSAGE_VOTE_DOMAIN_VERSION,
};

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
    inbox: Address,
    dest_rpc_url: String,
    token: CancellationToken,
) {
    tracing::info!(%validator, %inbox, "🛂 attestor-set watcher online");

    // `interval` fires immediately on the first tick, which is what performs the *initial* population:
    // `build_state` starts the OnChainValidator set empty (to avoid blocking core startup on the
    // destination RPC), so this first tick reads the real set + threshold and swaps them in.
    let mut tick = tokio::time::interval(Duration::from_secs(ATTESTOR_SET_POLL_SECS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Rate-limit pacing: every attestor in the fleet runs this poll on the same cadence against
    // the same (often shared, RPS-capped) destination endpoint, colliding with the fleet's
    // synchronized per-block fetches. When the provider rejects on rate limits, escalate this
    // loop's cooldown instead of colliding again next tick (observed live 2026-08-07, Chainstack
    // -32005 on usc-dev; mirrors the relayer's pacing module). Base the window on this loop's own
    // poll interval — a shorter window would expire before the next tick and never actually skip
    // a poll, so the fleet would keep colliding on the first few escalations (bugbot).
    let mut pacer = eth::RateLimitPacer::new(Duration::from_secs(ATTESTOR_SET_POLL_SECS));

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
                // Rate-limit pacing: skip the poll while a deferral window is active (armed only
                // by rate-limited failures; clean polls decay the level and are never slowed).
                if let Some(remaining) = pacer.deferring() {
                    tracing::debug!(remaining_ms = remaining.as_millis() as u64,
                        "⏸️ pacing attestor-set watcher — skipping poll");
                    continue;
                }
                // Fresh connection each poll — see the fn doc. A dead/transiently-unreachable RPC
                // fails only this tick and is retried on the next, instead of wedging the watcher.
                let (set, onchain_threshold, domain) = match tokio::time::timeout(
                    super::RPC_ATTEMPT_TIMEOUT,
                    fetch_destination_view(&dest_rpc_url, validator, inbox),
                )
                .await
                {
                    Ok(Ok(v)) => {
                        pacer.after(false);
                        v
                    }
                    Ok(Err(err)) => {
                        tracing::warn!(%validator, %err, "failed to read on-chain attestor set/threshold; will retry");
                        let rate_limited = eth::error_looks_rate_limited(&format!("{err:#}"));
                        pacer.after(rate_limited);
                        if rate_limited {
                            if let Some(window) = pacer.deferring() {
                                tracing::warn!(
                                    defer_ms = window.as_millis() as u64,
                                    "🧯 provider is rate limiting — deferring attestor-set polls"
                                );
                            }
                        }
                        continue;
                    }
                    Err(_) => {
                        tracing::warn!(
                            %validator,
                            timeout_secs = super::RPC_ATTEMPT_TIMEOUT.as_secs(),
                            "timed out reading on-chain attestor set/threshold; will retry"
                        );
                        continue;
                    }
                };
                apply_vote_domain(&state, domain);
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
    Ok(ret.into_iter().collect())
}

async fn read_threshold<P: Provider>(provider: &P, validator: Address) -> anyhow::Result<U256> {
    let ret = write_ability::abi::IVoteValidator::new(validator, provider)
        .threshold()
        .call()
        .await?;
    Ok(ret)
}

/// Read the validator's EIP-712 domain and the Inbox's validator, and build the message-vote domain
/// if they match what this build signs. The outer `Err` is an RPC failure (keep the last-known
/// domain and retry); the inner `Err` is a definite mismatch (stop signing).
async fn read_vote_domain<P: Provider>(
    provider: &P,
    validator: Address,
    inbox: Address,
) -> anyhow::Result<Result<MessageVoteDomain, String>> {
    let d = write_ability::abi::IVoteValidator::new(validator, provider)
        .eip712Domain()
        .call()
        .await
        .map_err(|err| anyhow::anyhow!("EOAValidator.eip712Domain(): {err}"))?;
    let inbox_validator = write_ability::abi::IInbox::new(inbox, provider)
        .defaultVoteValidator()
        .call()
        .await
        .map_err(|err| anyhow::anyhow!("Inbox.defaultVoteValidator(): {err}"))?;
    Ok(check_vote_domain(
        validator,
        inbox,
        &d.name,
        &d.version,
        d.chainId,
        d.verifyingContract,
        inbox_validator,
    ))
}

/// Pure half of [`read_vote_domain`], split out for tests.
fn check_vote_domain(
    validator: Address,
    inbox: Address,
    name: &str,
    version: &str,
    chain_id: U256,
    verifying_contract: Address,
    inbox_validator: Address,
) -> Result<MessageVoteDomain, String> {
    if name != MESSAGE_VOTE_DOMAIN_NAME || version != MESSAGE_VOTE_DOMAIN_VERSION {
        return Err(format!(
            "validator EIP-712 domain is ({name:?}, {version:?}), this attestor signs \
             ({MESSAGE_VOTE_DOMAIN_NAME:?}, {MESSAGE_VOTE_DOMAIN_VERSION:?})"
        ));
    }
    if verifying_contract != validator {
        return Err(format!(
            "validator EIP-712 verifyingContract is {verifying_contract}, expected {validator}"
        ));
    }
    if inbox_validator != validator {
        return Err(format!(
            "Inbox {inbox} hands votes to {inbox_validator}, not the configured validator {validator}"
        ));
    }
    let chain_id = u64::try_from(chain_id)
        .map_err(|_| format!("validator EIP-712 chainId {chain_id} does not fit in u64"))?;
    Ok(MessageVoteDomain {
        chain_id,
        validator,
        inbox,
    })
}

/// Install a freshly read domain. A transient read failure was already filtered out by the caller,
/// so `Err` here is a definite mismatch and clears the domain (fail closed).
fn apply_vote_domain(state: &MessageVoteState, domain: Result<MessageVoteDomain, String>) {
    let next = match domain {
        Ok(d) => Some(d),
        Err(reason) => {
            tracing::error!(%reason, "⛔ message-vote EIP-712 domain mismatch — message signing paused");
            None
        }
    };
    let mut current = state.vote_domain.write();
    if *current != next {
        if let Some(d) = &next {
            tracing::info!(
                chain_id = d.chain_id,
                validator = %d.validator,
                inbox = %d.inbox,
                "🔏 message-vote EIP-712 domain resolved"
            );
        }
        *current = next;
    }
}

/// Connect to the destination chain and read, over one connection, the validator's attestor set,
/// its quorum threshold and the message-vote domain. The set and threshold are trust-critical and
/// read together so the local view cannot apply a new set against a stale threshold or vice versa.
pub(super) async fn fetch_destination_view(
    dest_rpc_url: &str,
    validator: Address,
    inbox: Address,
) -> anyhow::Result<(HashSet<Address>, u64, Result<MessageVoteDomain, String>)> {
    let provider = ProviderBuilder::new()
        .connect(dest_rpc_url)
        .await
        .map_err(|err| anyhow::anyhow!("connect destination chain RPC: {err}"))?;
    let set = read_attestors(&provider, validator).await?;
    let threshold = read_threshold(&provider, validator).await?;
    let domain = read_vote_domain(&provider, validator, inbox).await?;
    Ok((set, u64::try_from(threshold).unwrap_or(u64::MAX), domain))
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: Address = Address::ZERO;

    #[test]
    fn vote_domain_requires_matching_validator_and_inbox_wiring() {
        use alloy::primitives::address;
        let validator = address!("00000000000000000000000000000000000000e1");
        let inbox = address!("00000000000000000000000000000000000001b0");
        let other = address!("00000000000000000000000000000000000000e2");
        let check = |name: &str, version: &str, verifying: Address, inbox_validator: Address| {
            check_vote_domain(
                validator,
                inbox,
                name,
                version,
                U256::from(42u64),
                verifying,
                inbox_validator,
            )
        };
        assert_eq!(
            check(
                MESSAGE_VOTE_DOMAIN_NAME,
                MESSAGE_VOTE_DOMAIN_VERSION,
                validator,
                validator
            ),
            Ok(MessageVoteDomain {
                chain_id: 42,
                validator,
                inbox
            })
        );
        assert!(check("Other", MESSAGE_VOTE_DOMAIN_VERSION, validator, validator).is_err());
        assert!(check(MESSAGE_VOTE_DOMAIN_NAME, "2", validator, validator).is_err());
        assert!(check(
            MESSAGE_VOTE_DOMAIN_NAME,
            MESSAGE_VOTE_DOMAIN_VERSION,
            other,
            validator
        )
        .is_err());
        assert!(
            check(
                MESSAGE_VOTE_DOMAIN_NAME,
                MESSAGE_VOTE_DOMAIN_VERSION,
                validator,
                other
            )
            .is_err(),
            "an Inbox wired to a different validator must stop signing"
        );
    }

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
