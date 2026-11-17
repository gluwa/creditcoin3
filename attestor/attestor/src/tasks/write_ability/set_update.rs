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
use std::time::{Duration, Instant};

use alloy::eips::BlockId;
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

/// Pin validator reads this many blocks behind head, so a load-balanced RPC backend that is a
/// block behind does not fail the calls with "header not found". A vote signed against a nonce an
/// update has just bumped is rejected on-chain, so the lag is safe.
const READ_LAG_BLOCKS: u64 = 2;

/// Re-warn about an unchanged deferral at most this often.
const DEFER_WARN_EVERY: Duration = Duration::from_secs(600);

/// The destination validator's own quorums, from `calculateRequiredVotes`: for the registered
/// subset we would propose, and for the full elected committee.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quorums {
    pub registered: U256,
    pub active: U256,
}

/// Why a proposal was held back: the registered subset's quorum is below the bar, or unreachable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deferral {
    pub registered: usize,
    pub active: usize,
    pub required: U256,
    pub quorum: U256,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Defer(Deferral),
    /// `weaker_validator`: the validator's full-committee quorum is below Creditcoin's.
    Pass {
        weaker_validator: bool,
    },
}

/// CSUB-2136 gate. The proposed set must keep the destination quorum at or above the lower of
/// Creditcoin's `calculate_threshold(active)` and the validator's own full-committee quorum (a
/// validator configured weaker than Creditcoin must still be able to rotate), and that quorum must
/// be reachable by the subset itself. The guarantee is on the signature count, not the set size: a
/// binding `minAttestorCount` can admit a smaller subset, but it then needs at least `required`
/// signers, all elected attestors. Deliberately strict: with 2/3+1 an elected attestor may stay
/// unregistered only when `active % 3 == 1` (and then just one); for other committee sizes every
/// elected attestor must register, otherwise the set stays as is (see [`Notices`]).
#[must_use]
pub fn gate(registered: usize, active: usize, quorums: &Quorums) -> Gate {
    let committee_active = u32::try_from(active).unwrap_or(u32::MAX);
    let committee = U256::from(attestor_primitives::calculate_threshold(committee_active));
    let required = committee.min(quorums.active);
    if quorums.registered >= required && quorums.registered <= U256::from(registered) {
        Gate::Pass {
            weaker_validator: quorums.active < committee,
        }
    } else {
        Gate::Defer(Deferral {
            registered,
            active,
            required,
            quorum: quorums.registered,
        })
    }
}

/// Keeps the proposer's warnings to one per change instead of one per tick.
#[derive(Default)]
struct Notices {
    deferral: Option<(Deferral, Instant)>,
    weaker_nonce: Option<U256>,
}

impl Notices {
    /// Whether to warn about this deferral: when it is new, or unchanged for [`DEFER_WARN_EVERY`].
    fn warn_deferral(&mut self, d: Deferral, now: Instant) -> bool {
        let repeat = self
            .deferral
            .is_some_and(|(prev, at)| prev == d && now.duration_since(at) < DEFER_WARN_EVERY);
        if !repeat {
            self.deferral = Some((d, now));
        }
        !repeat
    }

    fn clear_deferral(&mut self) {
        self.deferral = None;
    }

    /// Whether to warn about a weaker validator: once per update nonce.
    fn warn_weaker(&mut self, nonce: U256) -> bool {
        let first = self.weaker_nonce != Some(nonce);
        self.weaker_nonce = Some(nonce);
        first
    }
}

/// Result of one proposal cycle.
enum Cycle {
    NoChange,
    Deferred(Deferral),
    Vote {
        vote: SetUpdateVote,
        /// `(validator full-committee quorum, Creditcoin committee quorum)` when the former is lower.
        weaker_validator: Option<(U256, U256)>,
    },
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
/// set; if they diverge and the gate passes, return a signed vote to gossip. The cheap set
/// comparison runs first, and every validator read is pinned to one block so the set, nonce and
/// quorums are consistent.
async fn propose_once(
    cc3: &cc_client::Client,
    chain_key: u64,
    dest_rpc_url: &str,
    validator: Address,
    signer: &MessageSigner,
) -> Result<Cycle> {
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
        return Ok(Cycle::NoChange);
    }
    let provider = ProviderBuilder::new()
        .connect(dest_rpc_url)
        .await
        .map_err(|e| anyhow::anyhow!("connect destination chain RPC: {e}"))?;
    let contract = IVoteValidator::new(validator, &provider);
    let head = provider
        .get_block_number()
        .await
        .context("read destination block number")?;
    let block = BlockId::number(head.saturating_sub(READ_LAG_BLOCKS));

    let current = contract
        .attestors()
        .block(block)
        .call()
        .await
        .context("read EOAValidator.attestors()")?;
    if !set_needs_update(&current, &candidate) {
        return Ok(Cycle::NoChange);
    }

    let quorums = Quorums {
        registered: contract
            .calculateRequiredVotes(U256::from(candidate.len()))
            .block(block)
            .call()
            .await
            .context("read EOAValidator.calculateRequiredVotes(registered)")?,
        active: contract
            .calculateRequiredVotes(U256::from(active_count))
            .block(block)
            .call()
            .await
            .context("read EOAValidator.calculateRequiredVotes(active)")?,
    };
    let weaker_validator = match gate(candidate.len(), active_count, &quorums) {
        Gate::Defer(deferral) => return Ok(Cycle::Deferred(deferral)),
        Gate::Pass { weaker_validator } => weaker_validator.then(|| {
            let active = u32::try_from(active_count).unwrap_or(u32::MAX);
            (
                quorums.active,
                U256::from(attestor_primitives::calculate_threshold(active)),
            )
        }),
    };

    let nonce = contract
        .attestorSetUpdateNonce()
        .block(block)
        .call()
        .await
        .context("read EOAValidator.attestorSetUpdateNonce()")?;
    let chain_id = U256::from(
        provider
            .get_chain_id()
            .await
            .context("read destination chain id")?,
    );

    let vote = build_set_update_vote(signer, chain_key, validator, &candidate, chain_id, nonce)?;
    tracing::info!(
        proposed = candidate.len(),
        current = current.len(),
        nonce = %nonce,
        "🗳️ proposing attestor-set update — signing and gossiping"
    );
    Ok(Cycle::Vote {
        vote,
        weaker_validator,
    })
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
    let mut notices = Notices::default();

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
                    Ok(Ok(Cycle::Vote { vote, weaker_validator })) => {
                        pacer.after(false);
                        notices.clear_deferral();
                        if let Some((validator_quorum, committee_quorum)) = weaker_validator {
                            if notices.warn_weaker(U256::from_be_bytes(vote.nonce)) {
                                tracing::warn!(
                                    %validator_quorum, %committee_quorum,
                                    "destination validator's full-committee quorum is below the Creditcoin committee's"
                                );
                            }
                        }
                        // Bounded `try_send`: if the publish channel is full the vote is dropped and
                        // re-proposed next tick (set changes persist until an update lands), so a
                        // backed-up publisher never blocks this loop.
                        if publish_tx.try_send(vote).is_err() {
                            tracing::warn!("set-update vote publish channel full — will re-propose next tick");
                        }
                    }
                    Ok(Ok(Cycle::Deferred(d))) => {
                        pacer.after(false);
                        if notices.warn_deferral(d, Instant::now()) {
                            tracing::warn!(
                                registered = d.registered, active = d.active,
                                required = %d.required, quorum = %d.quorum,
                                "attestor-set update deferred — the registered subset would lower the delivery quorum; the destination set stays as is until more elected attestors register an EVM address"
                            );
                        } else {
                            tracing::debug!(registered = d.registered, active = d.active, "attestor-set update still deferred");
                        }
                    }
                    Ok(Ok(Cycle::NoChange)) => {
                        pacer.after(false);
                        notices.clear_deferral();
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

    /// Test-only mirror of `EOAValidator.calculateRequiredVotes` (the production code asks the
    /// contract): `max(n * numerator / 30 + addition, minimum)`.
    #[derive(Clone, Copy, Debug)]
    struct Cfg {
        numerator: u64,
        addition: u64,
        minimum: u64,
    }

    /// The default deployment: 20/30 + 1, floor 3 (the contract rejects a floor below 3).
    const TWO_THIRDS: Cfg = Cfg {
        numerator: 20,
        addition: 1,
        minimum: 3,
    };

    impl Cfg {
        fn votes(self, n: usize) -> u64 {
            (n as u64 * self.numerator / 30 + self.addition).max(self.minimum)
        }

        fn quorums(self, registered: usize, active: usize) -> Quorums {
            Quorums {
                registered: U256::from(self.votes(registered)),
                active: U256::from(self.votes(active)),
            }
        }

        /// `_validateQuorumConfig(n, ..)`: the set is large enough and its quorum reachable.
        fn reachable(self, n: usize) -> bool {
            let n = n as u64;
            n >= self.minimum && n * self.numerator / 30 + self.addition <= n
        }

        /// The same check plus the contract's "zero threshold" revert, which looks at the quorum
        /// *before* the floor and so is invisible through `calculateRequiredVotes`.
        fn accepts(self, n: usize) -> bool {
            self.reachable(n) && n as u64 * self.numerator / 30 + self.addition != 0
        }

        fn passes(self, registered: usize, active: usize) -> bool {
            matches!(
                gate(registered, active, &self.quorums(registered, active)),
                Gate::Pass { .. }
            )
        }
    }

    #[test]
    fn gate_never_lowers_the_committee_quorum() {
        let c = TWO_THIRDS;
        assert!(c.passes(3, 3));
        assert!(!c.passes(2, 3)); // quorum 3 > 2 registered
        assert!(c.passes(3, 4)); // quorum 3 either way
        assert!(!c.passes(2, 4));
        assert!(c.passes(6, 7)); // quorum 5 either way
        assert!(!c.passes(5, 7)); // quorum 4 < 5
        assert!(c.passes(9, 10) && !c.passes(8, 10));
        // 67 of 100 registered would give quorum 45 against the committee's 67.
        assert!(!c.passes(67, 100));
        assert!(c.passes(99, 100));
        assert!(!c.passes(98, 100));
        assert!(!c.passes(11, 12)); // 12 % 3 == 0: every elected attestor must register
    }

    #[test]
    fn default_config_tolerates_one_unregistered_attestor_only_when_active_is_1_mod_3() {
        for active in 3..=60usize {
            let c = TWO_THIRDS;
            assert!(c.passes(active, active), "active={active}");
            assert_eq!(
                c.passes(active - 1, active),
                active % 3 == 1,
                "active={active}"
            );
            assert!(!c.passes(active - 2, active), "active={active}");
        }
    }

    #[test]
    fn weaker_validator_rotates_without_lowering_its_own_quorum() {
        // 15/30 + 1 over 10 attestors: the validator needs 6, Creditcoin 7, so Creditcoin's bar is
        // out of reach; the validator is held to its own full-committee quorum.
        let weak = Cfg {
            numerator: 15,
            ..TWO_THIRDS
        };
        assert_eq!(
            gate(10, 10, &weak.quorums(10, 10)),
            Gate::Pass {
                weaker_validator: true
            }
        );
        assert!(!weak.passes(9, 10)); // quorum 5 < 6
                                      // A stricter validator keeps Creditcoin's bar (7 of 10).
        let strict = Cfg {
            numerator: 25,
            ..TWO_THIRDS
        };
        assert!(strict.passes(8, 10)); // quorum 7
        assert!(!strict.passes(7, 10)); // quorum 6
    }

    #[test]
    fn binding_floor_never_lowers_the_required_signers() {
        // Floor 8 over 10 attestors, 15/30: required = min(7, 8) = 7.
        let floored = Cfg {
            numerator: 15,
            minimum: 8,
            ..TWO_THIRDS
        };
        assert!(floored.passes(8, 10)); // needs all 8 to sign, above the committee's 7
        assert!(!floored.passes(7, 10)); // floor makes the quorum 8 > 7 registered
                                         // Odd committee: floor 5 over 9.
        let odd = Cfg {
            numerator: 15,
            minimum: 5,
            ..TWO_THIRDS
        };
        assert!(odd.passes(5, 9));
        assert!(!odd.passes(4, 9));
    }

    #[test]
    fn zero_fraction_config_is_the_one_contract_check_the_gate_cannot_see() {
        // 1/30 + 0 over 30 attestors: the fraction yields 1 vote, so the floor (3) decides. A
        // 3-attestor set passes the gate, but the contract rejects it ("zero threshold": 3/30 = 0
        // before the floor). Only a degenerate deployment hits this, and the effect is an update
        // that fails on submission, not a lower quorum.
        let degenerate = Cfg {
            numerator: 1,
            addition: 0,
            minimum: 3,
        };
        assert!(degenerate.passes(3, 30));
        assert!(degenerate.reachable(3) && !degenerate.accepts(3));
    }

    #[test]
    fn deferral_reports_the_counts_and_the_bar() {
        let Gate::Defer(d) = gate(67, 100, &TWO_THIRDS.quorums(67, 100)) else {
            panic!("67 of 100 must defer");
        };
        assert_eq!((d.registered, d.active), (67, 100));
        assert_eq!(
            (d.quorum, d.required),
            (U256::from(45u64), U256::from(67u64))
        );
    }

    #[test]
    fn notices_warn_once_per_change() {
        let d = Deferral {
            registered: 67,
            active: 100,
            required: U256::from(67u64),
            quorum: U256::from(45u64),
        };
        let t0 = Instant::now();
        let mut n = Notices::default();
        assert!(n.warn_deferral(d, t0));
        assert!(!n.warn_deferral(d, t0 + Duration::from_secs(60)));
        assert!(n.warn_deferral(d, t0 + DEFER_WARN_EVERY)); // periodic reminder
        let changed = Deferral {
            registered: 68,
            ..d
        };
        assert!(n.warn_deferral(changed, t0 + DEFER_WARN_EVERY));
        n.clear_deferral();
        assert!(n.warn_deferral(changed, t0 + DEFER_WARN_EVERY)); // recovered, then deferred again
        assert!(n.warn_weaker(U256::from(1u64)));
        assert!(!n.warn_weaker(U256::from(1u64)));
        assert!(n.warn_weaker(U256::from(2u64)));
    }

    mod props {
        use super::*;
        use proptest::prelude::*;

        /// Every configuration and committee size the contract accepts.
        fn setup() -> impl Strategy<Value = (Cfg, usize, usize)> {
            (0u64..=30, 0u64..=5, 3u64..=20, 3usize..=120)
                .prop_map(|(numerator, addition, minimum, active)| {
                    (
                        Cfg {
                            numerator,
                            addition,
                            minimum,
                        },
                        active,
                    )
                })
                .prop_filter("contract accepts the full committee", |(c, a)| {
                    c.accepts(*a)
                })
                .prop_flat_map(|(c, a)| (Just(c), Just(a), 1..=a))
        }

        proptest! {
            /// Safety: whatever passes still needs at least the required signers, and the contract
            /// would accept the resulting set's size and quorum (`submitAttestorSetUpdate`
            /// revalidates the config). The zero-threshold revert is covered by a unit test.
            #[test]
            fn passing_sets_keep_the_quorum_and_are_accepted((c, active, registered) in setup()) {
                if c.passes(registered, active) {
                    let required = c.votes(active).min(u64::from(
                        attestor_primitives::calculate_threshold(active as u32),
                    ));
                    prop_assert!(c.votes(registered) >= required);
                    prop_assert!(c.reachable(registered));
                }
            }

            /// Liveness: full registration always passes.
            #[test]
            fn full_registration_always_passes((c, active, _r) in setup()) {
                prop_assert!(c.passes(active, active));
            }

            /// Reachability agrees with the contract: a subset it would reject never passes.
            #[test]
            fn contract_rejected_sets_never_pass((c, active, registered) in setup()) {
                if !c.reachable(registered) {
                    prop_assert!(!c.passes(registered, active));
                }
            }
        }
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
