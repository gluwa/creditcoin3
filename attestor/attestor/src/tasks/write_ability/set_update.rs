//! Attestor-set-update proposer (write-ability P2-8, off-chain half).
//!
//! Every poll, this compares the destination `EOAValidator`'s current attestor set against the
//! *elected* attestors that have registered a write-ability EVM address (the on-chain registry from
//! the `set_attestor_evm_address` extrinsic). When they differ, the attestor signs the canonical
//! update digest and gossips a [`SetUpdateVote`] on
//! [`attestor_set_update_topic`](write_ability::protocol::attestor_set_update_topic).
//!
//! Attestors only **propose** (sign + gossip) — they do not submit on-chain. The relayer snoops the
//! topic, aggregates a threshold of signatures, and calls `submitAttestorSetUpdate` (it already owns
//! destination-chain delivery, gas, and nonce management). This mirrors the message-delivery flow
//! exactly: attestors sign + gossip, relayer aggregates + delivers.
//!
//! Determinism is the crux: every attestor must sign over the **byte-identical** `newAttestors`
//! array and nonce, or the relayer can't aggregate their signatures. We enforce a canonical
//! (ascending) address ordering ([`canonical_attestor_order`]) and sign over the validator's current
//! `attestorSetUpdateNonce` — if an update lands, the nonce bumps and stale in-flight votes are
//! naturally rejected on-chain (the replay/rollback protection working as intended).

use std::sync::Arc;
use std::time::Duration;

use alloy::primitives::{Address, U256};
use alloy::providers::{Provider, ProviderBuilder};
use anyhow::{Context, Result};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use write_ability::abi::IVoteValidator;
use write_ability::envelope::SetUpdateVote;
use write_ability::hash::{attestor_set_update_digest, canonical_attestor_order};

use super::signing::MessageSigner;

/// How often to re-check whether the elected set diverged from the on-chain validator set. Set
/// changes only happen on epoch rotation, so a slow poll keeps RPC load negligible while bounding
/// how long a diverged set goes un-proposed.
const SET_UPDATE_POLL_SECS: u64 = 60;

/// The destination validator's threshold configuration, read from the contract: it is set at
/// deploy time, so the proposer must not assume `2N/3 + 1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThresholdParams {
    pub numerator: U256,
    pub addition: U256,
    pub denominator: U256,
    pub min_attestor_count: U256,
}

impl ThresholdParams {
    /// Mirrors `EOAValidator.threshold()` for a set of `n` attestors.
    #[must_use]
    pub fn threshold(&self, n: usize) -> U256 {
        let scaled = U256::from(n).saturating_mul(self.numerator);
        let t = scaled
            .checked_div(self.denominator)
            .map_or(U256::MAX, |q| q.saturating_add(self.addition));
        t.max(self.min_attestor_count)
    }
}

/// The quorum the proposed set must keep: the lower of Creditcoin's `calculate_threshold(active)`
/// and what this validator already requires of the full committee. A validator configured weaker
/// than Creditcoin must still be able to rotate, so it is held to its own full-committee quorum.
#[must_use]
pub fn required_quorum(active: usize, params: &ThresholdParams) -> U256 {
    U256::from(attestor_primitives::calculate_threshold(active as u32))
        .min(params.threshold(active))
}

/// Whether proposing the registered subset keeps the destination quorum at or above
/// [`required_quorum`] (CSUB-2136), and reachable by the subset itself. The guarantee is on the
/// signature count, not the set size: a binding `minAttestorCount` can admit a smaller subset, but
/// it then needs at least `required_quorum` signers, all of them elected attestors.
#[must_use]
pub fn enough_registered(registered: usize, active: usize, params: &ThresholdParams) -> bool {
    let quorum = params.threshold(registered);
    quorum >= required_quorum(active, params) && quorum <= U256::from(registered)
}

/// Whether the destination validator's `current` set differs from our `candidate` set, compared as
/// **sets** (order- and duplicate-insensitive) — the trigger to gossip an update. `candidate` is
/// expected pre-canonicalized; the set comparison makes the decision robust regardless.
#[must_use]
pub fn set_needs_update(current: &[Address], candidate: &[Address]) -> bool {
    use std::collections::BTreeSet;
    let current: BTreeSet<&Address> = current.iter().collect();
    let candidate: BTreeSet<&Address> = candidate.iter().collect();
    current != candidate
}

/// Build a signed [`SetUpdateVote`]. `new_attestors` is canonicalized (sorted, de-duped) before
/// hashing so every attestor signs identical bytes; `validator` is the target `EOAValidator` (the
/// contract binds its own address into the preimage), `chain_id` is the destination `block.chainid`
/// and `nonce` the validator's current `attestorSetUpdateNonce`.
pub fn build_set_update_vote(
    signer: &MessageSigner,
    chain_key: u64,
    validator: Address,
    new_attestors: &[Address],
    chain_id: U256,
    nonce: U256,
) -> Result<SetUpdateVote> {
    let canonical = canonical_attestor_order(new_attestors);
    let digest = attestor_set_update_digest(validator, &canonical, chain_id, nonce);
    let signature = signer
        .sign(&digest)
        .context("sign attestor-set-update digest")?;
    Ok(SetUpdateVote {
        chain_key,
        new_attestors: canonical.iter().map(|a| a.into_array()).collect(),
        nonce: nonce.to_be_bytes::<32>(),
        signer: signer.address().into_array(),
        signature,
    })
}

/// One proposal cycle: read the elected set's EVM addresses (registry) and the validator's current
/// set/nonce/chain-id; if they diverge, return a signed vote to gossip, else `Ok(None)`.
async fn propose_once(
    cc3: &cc_client::Client,
    chain_key: u64,
    dest_rpc_url: &str,
    validator: Address,
    signer: &MessageSigner,
) -> Result<Option<SetUpdateVote>> {
    // Candidate = EVM addresses of the currently-elected attestors that have registered one.
    let (registered, active_count) = cc3
        .active_attestor_evm_addresses(chain_key)
        .await
        .context("read elected attestors' EVM addresses")?;
    let candidate: Vec<Address> = registered
        .into_iter()
        .map(|h| Address::from_slice(h.as_bytes()))
        .collect();
    let candidate = canonical_attestor_order(&candidate);
    if candidate.is_empty() {
        // No elected attestor has registered an EVM address yet — nothing to propose. (Proposing an
        // empty set would brick the validator.)
        return Ok(None);
    }
    let provider = ProviderBuilder::new()
        .connect(dest_rpc_url)
        .await
        .map_err(|e| anyhow::anyhow!("connect destination chain RPC: {e}"))?;
    let contract = IVoteValidator::new(validator, &provider);

    let params = ThresholdParams {
        numerator: contract
            .thresholdNumerator()
            .call()
            .await
            .context("read EOAValidator.thresholdNumerator()")?,
        addition: contract
            .thresholdAddition()
            .call()
            .await
            .context("read EOAValidator.thresholdAddition()")?,
        denominator: contract
            .THRESHOLD_DENOMINATOR()
            .call()
            .await
            .context("read EOAValidator.THRESHOLD_DENOMINATOR()")?,
        min_attestor_count: contract
            .minAttestorCount()
            .call()
            .await
            .context("read EOAValidator.minAttestorCount()")?,
    };
    // Defer while the registered subset's quorum would be lower than the committee's (CSUB-2136).
    let weaker = params.threshold(active_count)
        < U256::from(attestor_primitives::calculate_threshold(
            active_count as u32,
        ));
    if !enough_registered(candidate.len(), active_count, &params) {
        tracing::debug!(
            registered = candidate.len(),
            active = active_count,
            "deferring set-update proposal — registered subset would lower the delivery quorum"
        );
        return Ok(None);
    }

    let current = contract
        .attestors()
        .call()
        .await
        .context("read EOAValidator.attestors()")?;
    if !set_needs_update(&current, &candidate) {
        return Ok(None);
    }

    let nonce = contract
        .attestorSetUpdateNonce()
        .call()
        .await
        .context("read EOAValidator.attestorSetUpdateNonce()")?;
    let chain_id = U256::from(
        provider
            .get_chain_id()
            .await
            .context("read destination chain id")?,
    );

    if weaker {
        tracing::warn!(
            validator_quorum = %params.threshold(active_count),
            committee_quorum = attestor_primitives::calculate_threshold(active_count as u32),
            "destination validator's full-committee quorum is below the Creditcoin committee's"
        );
    }

    let vote = build_set_update_vote(signer, chain_key, validator, &candidate, chain_id, nonce)?;
    tracing::info!(
        proposed = candidate.len(),
        current = current.len(),
        nonce = %nonce,
        "🗳️ proposing attestor-set update — signing and gossiping"
    );
    Ok(Some(vote))
}

/// Watch for elected-vs-validator set divergence and gossip a signed update vote when it appears,
/// until `token` fires. Best-effort: read/connection failures are logged and retried next tick.
/// Connects to the destination chain per-poll for the same self-healing reason as
/// [`super::attestor_set::watch`].
#[allow(clippy::too_many_arguments)]
pub async fn run_proposer(
    cc3: Arc<cc_client::Client>,
    chain_key: u64,
    dest_rpc_url: String,
    validator: Address,
    signer: MessageSigner,
    publish_tx: mpsc::Sender<SetUpdateVote>,
    token: CancellationToken,
) {
    tracing::info!(%validator, "🗳️ attestor-set-update proposer online");
    let mut tick = tokio::time::interval(Duration::from_secs(SET_UPDATE_POLL_SECS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Rate-limit pacing — see the attestor-set watcher: same shared-endpoint collision, same
    // damper. Only genuine rate-limit rejections escalate; timeouts and other errors keep the
    // base cadence. Base the window on this loop's own poll interval, same reasoning as the
    // attestor-set watcher (bugbot).
    let mut pacer = eth::RateLimitPacer::new(Duration::from_secs(SET_UPDATE_POLL_SECS));

    loop {
        tokio::select! {
            () = token.cancelled() => {
                tracing::info!("🛑 attestor-set-update proposer exiting on cancel");
                return;
            }
            _ = tick.tick() => {
                // Ensure this attestor's own EVM address is registered on-chain, every cycle. The
                // startup registration in `lib.rs` is best-effort and one-shot; re-checking here
                // recovers not only from a transient startup RPC blip (review P2-8 #2) but also from
                // the mapping later disappearing while the process runs — e.g. a chain-removal purge
                // (bugbot: don't latch on first success). Idempotent and cheap: `register_evm_address`
                // reads the on-chain value first and only submits an extrinsic when it is missing or
                // stale, so a steady state is a single storage read per cycle. Runs unconditionally,
                // ahead of the destination-pacing check below: it only talks to the Creditcoin chain
                // (`cc3`), never the rate-limited destination RPC, so destination throttling must not
                // also stall these registration retries (bugbot).
                super::register_evm_address(&cc3, &signer, chain_key).await;

                // Rate-limit pacing: skip the destination-RPC part of the cycle while a deferral
                // window is active (armed only by rate-limited failures; clean cycles decay the
                // level and are never slowed).
                if let Some(remaining) = pacer.deferring() {
                    tracing::debug!(remaining_ms = remaining.as_millis() as u64,
                        "⏸️ pacing set-update proposer — skipping cycle");
                    continue;
                }

                match tokio::time::timeout(
                    super::RPC_ATTEMPT_TIMEOUT,
                    propose_once(&cc3, chain_key, &dest_rpc_url, validator, &signer),
                )
                .await
                {
                    Ok(Ok(Some(vote))) => {
                        pacer.after(false);
                        // Bounded `try_send`: if the publish channel is full the vote is dropped and
                        // re-proposed next tick (set changes persist until an update lands), so a
                        // backed-up publisher never blocks this loop.
                        if publish_tx.try_send(vote).is_err() {
                            tracing::warn!("set-update vote publish channel full — will re-propose next tick");
                        }
                    }
                    Ok(Ok(None)) => {
                        pacer.after(false);
                    }
                    Ok(Err(err)) => {
                        tracing::warn!(%err, "attestor-set-update proposal cycle failed; will retry");
                        let rate_limited = eth::error_looks_rate_limited(&format!("{err:#}"));
                        pacer.after(rate_limited);
                        if rate_limited {
                            if let Some(window) = pacer.deferring() {
                                tracing::warn!(
                                    defer_ms = window.as_millis() as u64,
                                    "🧯 provider is rate limiting — deferring set-update cycles"
                                );
                            }
                        }
                    }
                    Err(_) => {
                        tracing::warn!(
                            timeout_secs = super::RPC_ATTEMPT_TIMEOUT.as_secs(),
                            "attestor-set-update proposal cycle timed out; will retry"
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::signing::recover_signer;
    use super::*;
    use alloy::primitives::address;

    /// `EOAValidator` as deployed by default: 20/30 + 1, no effective minimum.
    fn two_thirds() -> ThresholdParams {
        ThresholdParams {
            numerator: U256::from(20u64),
            addition: U256::from(1u64),
            denominator: U256::from(30u64),
            min_attestor_count: U256::from(1u64),
        }
    }

    #[test]
    fn enough_registered_never_lowers_the_committee_quorum() {
        let p = two_thirds();
        assert!(enough_registered(1, 1, &p));
        assert!(!enough_registered(1, 2, &p));
        assert!(enough_registered(2, 2, &p));
        assert!(!enough_registered(2, 3, &p));
        assert!(enough_registered(3, 3, &p));
        assert!(!enough_registered(2, 4, &p));
        assert!(enough_registered(3, 4, &p)); // quorum 3 either way
        assert!(!enough_registered(4, 7, &p));
        assert!(!enough_registered(5, 7, &p)); // quorum 4 < 5
        assert!(enough_registered(6, 7, &p)); // quorum 5 either way
                                              // 67 of 100 registered would give quorum 45 against the committee's 67.
        assert!(!enough_registered(67, 100, &p));
        assert!(enough_registered(99, 100, &p));
        assert!(!enough_registered(98, 100, &p));
    }

    #[test]
    fn enough_registered_uses_the_validators_configured_threshold() {
        // A validator deployed at 15/30 + 1: with the 2N/3 + 1 assumption 3 of 4 would pass, but
        // its real quorum for 3 attestors is 2, below the committee's 3.
        let half = ThresholdParams {
            numerator: U256::from(15u64),
            ..two_thirds()
        };
        assert!(!enough_registered(3, 4, &half));
        // A stricter validator (25/30 + 1) clears the committee quorum with fewer registrations.
        let strict = ThresholdParams {
            numerator: U256::from(25u64),
            ..two_thirds()
        };
        assert!(enough_registered(8, 10, &strict)); // quorum 7 >= committee 7
    }

    #[test]
    fn weaker_validator_can_still_rotate_without_lowering_its_own_quorum() {
        // 15/30 + 1 over 10 attestors: the validator needs 6, Creditcoin 7. Creditcoin's bar is
        // out of reach for this deployment, so it is held to its own full-committee quorum.
        let half = ThresholdParams {
            numerator: U256::from(15u64),
            ..two_thirds()
        };
        assert_eq!(required_quorum(10, &half), U256::from(6u64));
        assert!(enough_registered(10, 10, &half)); // full registration rotates
        assert!(!enough_registered(9, 10, &half)); // quorum 5 < 6
                                                   // A stricter validator keeps Creditcoin's bar.
        let strict = ThresholdParams {
            numerator: U256::from(25u64),
            ..two_thirds()
        };
        assert_eq!(required_quorum(10, &strict), U256::from(7u64));
    }

    #[test]
    fn binding_min_attestor_count_never_lowers_the_required_signers() {
        // Floor 8 over 10 attestors with weak 15/30 params: required quorum is min(7, 8) = 7.
        let floored = ThresholdParams {
            numerator: U256::from(15u64),
            min_attestor_count: U256::from(8u64),
            ..two_thirds()
        };
        assert_eq!(required_quorum(10, &floored), U256::from(7u64));
        assert!(enough_registered(8, 10, &floored)); // needs all 8 to sign, above the committee's 7
        assert!(!enough_registered(7, 10, &floored)); // floor makes the quorum 8 > 7 registered
                                                      // Odd committee, floor 5 over 9 attestors (committee quorum 7, validator quorum 5).
        let odd = ThresholdParams {
            numerator: U256::from(15u64),
            min_attestor_count: U256::from(5u64),
            ..two_thirds()
        };
        assert_eq!(required_quorum(9, &odd), U256::from(5u64));
        assert!(enough_registered(5, 9, &odd)); // 5-of-5, same signer count as the full set needs
        assert!(!enough_registered(4, 9, &odd));
        // Whatever passes, the signers it needs never fall below the required quorum.
        for active in 1..=40usize {
            for registered in 1..=active {
                if enough_registered(registered, active, &odd) {
                    assert!(odd.threshold(registered) >= required_quorum(active, &odd));
                }
            }
        }
    }

    #[test]
    fn enough_registered_rejects_an_unreachable_destination_quorum() {
        // minAttestorCount above the registered count: the floor makes the quorum unreachable.
        let floor3 = ThresholdParams {
            min_attestor_count: U256::from(3u64),
            ..two_thirds()
        };
        assert!(!enough_registered(2, 3, &floor3)); // quorum 3 > 2 registered
        assert!(enough_registered(3, 3, &floor3));
    }

    #[test]
    fn threshold_params_mirror_the_contract_formula() {
        let p = two_thirds();
        assert_eq!(p.threshold(100), U256::from(67u64));
        assert_eq!(p.threshold(4), U256::from(3u64));
        let zero_den = ThresholdParams {
            denominator: U256::ZERO,
            ..p
        };
        assert_eq!(zero_den.threshold(4), U256::MAX);
    }

    #[test]
    fn set_needs_update_is_order_and_dup_insensitive() {
        let a = address!("00000000000000000000000000000000000000aa");
        let b = address!("00000000000000000000000000000000000000bb");
        // Same members, different order / dups → no update needed.
        assert!(!set_needs_update(&[a, b], &[b, a]));
        assert!(!set_needs_update(&[a, b, a], &[b, a]));
        // Added member → update needed.
        let c = address!("00000000000000000000000000000000000000cc");
        assert!(set_needs_update(&[a, b], &[a, b, c]));
        // Removed member → update needed.
        assert!(set_needs_update(&[a, b, c], &[a, b]));
        // Empty vs non-empty.
        assert!(set_needs_update(&[], &[a]));
    }

    #[test]
    fn build_vote_signs_canonical_digest_and_recovers_to_signer() {
        let signer = MessageSigner::from_seed(&[9u8; 32]).unwrap();
        let a = address!("00000000000000000000000000000000000000cc");
        let b = address!("00000000000000000000000000000000000000aa");
        let chain_id = U256::from(11_155_111u64);
        let nonce = U256::from(3u64);

        let validator = address!("00000000000000000000000000000000000000e1");
        let vote = build_set_update_vote(&signer, 2, validator, &[a, b], chain_id, nonce).unwrap();

        // Addresses are stored canonically (sorted), not in input order.
        let canonical = canonical_attestor_order(&[a, b]);
        let expected: Vec<[u8; 20]> = canonical.iter().map(|a| a.into_array()).collect();
        assert_eq!(vote.new_attestors, expected);
        assert_eq!(vote.signer, signer.address().into_array());
        assert_eq!(vote.nonce, nonce.to_be_bytes::<32>());

        // The signature recovers to our address over the digest the relayer will reconstruct.
        let digest = attestor_set_update_digest(validator, &canonical, chain_id, nonce);
        let recovered = recover_signer(&digest, &vote.signature).unwrap();
        assert_eq!(recovered, signer.address());
    }
}
