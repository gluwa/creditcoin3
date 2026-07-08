//! Validation task — consumes quorums from the pool, runs continuity-proof validation **as the
//! submitter only**, and ships winning attestations on chain.
//!
//! Design notes:
//!
//! - No CC3 stream of its own. We watch `shared.latest_finalized_rx` (driven by production)
//!   for `BlockAttested(h ≥ X)` patterns. One CC3 subscription, system-wide.
//! - The in-flight submission state is `Option<JoinHandle<Submission>>`, awaited via
//!   `futures::future::OptionFuture` in the main `select!` — no bespoke future type.
//! - Continuity-proof checks run **once**, on our locally-cached proof. Peers do no proof
//!   work on incoming votes.
//! - Pool consumption is `pool_rx.recv().await` (Notify-backed), not a `Stream::next` poll —
//!   no manual `Waker` plumbing.
//! - No `ctrl_c` arms; cancellation flows through `shared.token`.

use std::sync::Arc;

use rand::seq::SliceRandom as _;
use rand::SeedableRng as _;

use attestor_pool::{Permit, Quorum, Receiver, SignedQuorum, Vote};
use attestor_primitives::{Digest, Height};
use bls_signatures::Serialize as _;

use crate::error::Error;
use crate::shared::Shared;

pub async fn run(shared: Arc<Shared>, pool_rx: Receiver) -> Result<(), Error> {
    let mut in_flight: Option<tokio::task::JoinHandle<Submission>> = None;
    // Height of the in-flight submission. Used to filter duplicate-height quorums the pool
    // may surface while we're mid-submit (otherwise we'd stash twice for the same height).
    let mut in_flight_height: Option<Height> = None;

    loop {
        // Compose an OptionFuture so we only poll the submission handle when one is in flight.
        let submission_fut = futures::future::OptionFuture::from(in_flight.as_mut());

        tokio::select! {
            biased;
            _ = shared.token.cancelled() => return Ok(()),

            // In-flight submission finished.
            Some(joined) = submission_fut => {
                in_flight = None;
                in_flight_height = None;
                let submission = joined.map_err(Error::TaskJoin)?;
                handle_submission_result(
                    &shared,
                    &pool_rx,
                    &mut in_flight,
                    &mut in_flight_height,
                    submission,
                ).await;
            }

            // New quorum from the pool. Match on `Option` directly so a closed pool surfaces
            // explicitly — using the `Some(_) = pool_rx.recv()` shape silently disables this
            // arm forever when the last `Sender` drops, freezing the task on the cancellation
            // arm with no log.
            maybe_work = pool_rx.recv() => {
                let Some((quorum, permit)) = maybe_work else {
                    tracing::info!("📮 attestation pool closed — exiting validation loop");
                    return Ok(());
                };
                // If we're already submitting at this height, drop this fork on the floor — the
                // height is already locked by the in-flight submission, and re-stashing the same
                // height is wasted work. Use `mark_skipped`, NOT `mark_valid`: this fork was never
                // run through `aggregate_and_validate`, and `mark_valid` would overwrite
                // `digest_local` with this unvalidated (possibly different) digest and re-lock the
                // height. `mark_skipped` just removes the stray fork.
                if Some(quorum.height) == in_flight_height {
                    tracing::debug!(
                        height = quorum.height,
                        digest = ?quorum.digest,
                        "🪞 duplicate-height quorum while submitting — discarding"
                    );
                    pool_rx.mark_skipped(permit);
                    continue;
                }
                handle_quorum(
                    &shared,
                    &pool_rx,
                    &mut in_flight,
                    &mut in_flight_height,
                    quorum,
                    permit,
                ).await?;
            }
        }
    }
}

// ----------------------------------- [ Handle a new quorum ] ---------------------------------- //

async fn handle_quorum(
    shared: &Arc<Shared>,
    pool_rx: &Receiver,
    in_flight: &mut Option<tokio::task::JoinHandle<Submission>>,
    in_flight_height: &mut Option<Height>,
    quorum: Quorum,
    permit: Permit,
) -> Result<(), Error> {
    let height = quorum.height;
    let digest = quorum.digest;
    tracing::info!(
        ?digest,
        height,
        votes = quorum.votes.len(),
        "🗳️ quorum reached"
    );

    let digest_local = pool_rx.digest_local();
    let agg = match aggregate_and_validate(shared, &quorum, digest_local).await {
        Ok(a) => a,
        Err(ValidationError::Invalid) => {
            tracing::warn!(?digest, height, "⛔ quorum invalid — dropping fork");
            pool_rx.mark_invalid(permit);
            shared.metrics.increase_invalid_attestation_count();
            return Ok(());
        }
        Err(ValidationError::NoLocalProof) => {
            // Quorum reached on a fork we don't have a proof for. We can't submit without
            // someone's continuity proof. Skip just this fork (without locking the height) so a
            // different fork at the same height can still be validated; we'll keep listening for
            // the matching-digest quorum we're producing locally.
            tracing::warn!(
                ?digest,
                height,
                "🙈 quorum digest has no matching local proof — skipping submission"
            );
            pool_rx.mark_skipped(permit);
            return Ok(());
        }
        Err(ValidationError::InsufficientVotes {
            needed,
            got,
            target_sample_size,
        }) => {
            // The live threshold is higher than the pool's configured quorum target — submitting
            // now would hit `MajorityNotReached` at runtime level. Two things must happen:
            //
            //   1. Raise the pool's target to the live value so it stops re-yielding this same
            //      sub-threshold fork on the next `recv()` (otherwise we busy-loop re-validating
            //      it). `note_target_sample_size_change` is idempotent with production's own
            //      handler.
            //   2. Drop the permit *without* `mark_valid`. `mark_valid` would `split_off` the
            //      fork (discarding the votes we still need) and lock the height, rejecting the
            //      very gossiped votes required to reach the bigger threshold. Dropping the
            //      permit leaves the fork in place so it re-surfaces once enough votes arrive.
            //
            // We never submitted, so no fees were burned and no on-chain race was created.
            tracing::warn!(
                ?digest,
                height,
                needed,
                got,
                "🗳️ insufficient votes for current threshold — raising pool target, awaiting more votes"
            );
            shared
                .pool_send
                .note_target_sample_size_change(target_sample_size);
            // `Permit` is `Copy` with no Drop side effect; explicitly discard it (it's
            // `#[must_use]`) without `mark_valid`/`mark_invalid` so the fork stays in the pool.
            let _ = permit;
            return Ok(());
        }
        Err(ValidationError::External(e)) => return Err(e),
    };

    if in_flight.is_none() {
        pool_rx.mark_valid(permit);
        *in_flight_height = Some(agg.height);
        *in_flight = Some(spawn_submission(shared.clone(), agg));
    } else {
        tracing::info!(?digest, height, "🗃️ stash for later");
        pool_rx.mark_for_later(
            permit,
            SignedQuorum {
                height: agg.height,
                digest: agg.digest,
                signed: agg.attestation_signed,
                votes: agg.votes,
            },
        );
    }
    Ok(())
}

// ----------------------------- [ Post-submission finalization ] ------------------------------- //

async fn handle_submission_result(
    shared: &Arc<Shared>,
    pool_rx: &Receiver,
    in_flight: &mut Option<tokio::task::JoinHandle<Submission>>,
    in_flight_height: &mut Option<Height>,
    submission: Submission,
) {
    let Submission { height, outcome } = submission;
    let unresolved = matches!(outcome, Outcome::Unresolved);
    match outcome {
        Outcome::Eligible {
            result: Ok(events), ..
        } => {
            if let Ok(Some(_)) = events
                .all_events_in_block()
                .find_last::<cc_client::cc3::attestation::events::BlockAttested>()
            {
                tracing::info!(height, "✅ submitted on-chain");
            }
        }
        Outcome::Eligible {
            result: Err(subxt::Error::Runtime(subxt::error::DispatchError::Module(err))),
            votes,
        } => match err.as_root_error::<cc_client::cc3::Error>() {
            Ok(cc_client::cc3::Error::Attestation(
                cc_client::cc3::attestation::Error::AttestationExists,
            )) => {
                tracing::info!(height, "✅ already submitted (we lost the race)");
            }
            Ok(cc_client::cc3::Error::Attestation(
                cc_client::cc3::attestation::Error::MajorityNotReached,
            )) => {
                // The preemptive threshold check in `aggregate_and_validate` normally catches
                // this before submission. Reaching here means the chain's `target_sample_size`
                // flipped *between* our fetch and the runtime's verification (≤1-block race).
                // Unlock the height so a future quorum can form from gossiped votes; do not
                // re-inject (the votes we held are already gone, and gossip will repopulate).
                let _ = votes;
                tracing::warn!(
                    height,
                    "🔁 MajorityNotReached at runtime — chain-race window; unlocking height"
                );
                shared.pool_send.note_majority_not_reached(height);
            }
            _ => {
                // A real runtime rejection (e.g. a continuity-proof error). Our extrinsic did
                // not land, so unlock the height: leaving `locally_validated_height` set would
                // reject all future votes there until some unrelated recovery. A genuinely bad
                // fork gets re-validated and tombstoned (`mark_invalid`) on the retry, so this
                // can't loop.
                tracing::warn!(
                    height,
                    ?err,
                    "⛔ runtime rejected submission — unlocking height"
                );
                shared.pool_send.note_majority_not_reached(height);
            }
        },
        Outcome::Eligible {
            result: Err(subxt::Error::Transaction(subxt::error::TransactionError::Invalid(_))),
            ..
        } => {
            // Benign race: another attestor's extrinsic landed first, so the runtime invalidated
            // ours after broadcast (usually a stale nonce or duplicate-height check). Demote to
            // info so Grafana doesn't light up on a known harmless outcome.
            tracing::info!(
                height,
                "✅ already submitted (lost the race after broadcast)"
            );
        }
        Outcome::Eligible {
            result: Err(err), ..
        } => {
            // RPC/transport failure: the extrinsic didn't land. Unlock the height so a future
            // quorum can retry rather than stay stuck behind the validation lock.
            tracing::warn!(height, ?err, "⛔ submission rpc error — unlocking height");
            shared.pool_send.note_majority_not_reached(height);
        }
        Outcome::Finalized => {
            tracing::info!(height, "✅ finalized externally");
        }
        Outcome::Unresolved => {
            tracing::warn!(
                height,
                "❓ submission outcome unresolved — unlocking height unless it finalizes shortly"
            );
        }
    }

    // Wait briefly for this height to finalize on chain before pulling the next stash.
    wait_finalized(shared, height).await;

    // Unresolved outcome (timeout, txpool rejection, chilled skip, rpc failure): if the height
    // *still* hasn't finalized after the bounded wait, clear the local validation lock. Leaving
    // it set would reject every future vote at this height, blocking same-height retries until
    // an unrelated recovery path happened to clear it.
    if unresolved {
        let finalized = shared
            .latest_finalized_rx
            .borrow()
            .map(|info| info.height >= height)
            .unwrap_or(false);
        if !finalized {
            tracing::warn!(
                height,
                "🔓 height not finalized after unresolved submission — unlocking"
            );
            shared.pool_send.note_majority_not_reached(height);
        }
    }

    // Stashed quorums were validated against the threshold / active set *at stash time*; either
    // may have changed while the previous submission was in flight. Revalidate before
    // submitting — a stale aggregate is guaranteed to be rejected at runtime level
    // (`MajorityNotReached` / `InvalidBlsSignature`) and would only burn fees. Stale stashes are
    // dropped (their height is unlocked so a fresh quorum can form from gossip) and we move on
    // to the next stashed height.
    while let Some(stashed) = pool_rx.take_next_validated() {
        let stash_height = stashed.height;
        let stash_digest = stashed.digest;
        match revalidate_stashed(shared, stashed).await {
            Some(agg) => {
                tracing::info!(
                    digest = ?stash_digest, height = stash_height,
                    "🛫 submitting pre-validated stash"
                );
                *in_flight_height = Some(agg.height);
                *in_flight = Some(spawn_submission(shared.clone(), agg));
                break;
            }
            None => {
                tracing::warn!(
                    digest = ?stash_digest, height = stash_height,
                    "🗑️ stashed quorum went stale while waiting — dropping and unlocking height"
                );
                shared.pool_send.note_majority_not_reached(stash_height);
            }
        }
    }
}

/// Re-check a stashed quorum against the *live* chain state immediately before submission:
///
/// 1. Drop votes whose signer left the active attestor set (the runtime filters `attestors` to
///    `ActiveAttestors`; a stale signer in the aggregate yields `InvalidBlsSignature`).
/// 2. Re-fetch `target_sample_size` and require the surviving vote count to still satisfy the
///    live threshold (otherwise the runtime rejects with `MajorityNotReached`).
///
/// If signers were dropped but the quorum still stands, the BLS aggregate and attestor list are
/// rebuilt from the surviving votes. Returns `None` when the quorum no longer satisfies the live
/// threshold (or the threshold fetch was cancelled at shutdown) — the caller drops the stash and
/// unlocks the height.
async fn revalidate_stashed(shared: &Arc<Shared>, stashed: SignedQuorum) -> Option<Aggregated> {
    let chain_key = shared.chain_key;
    let target_sample_size =
        crate::retry::with_retries(&shared.cc3, &shared.token, |cc3| async move {
            cc3.target_sample_size(chain_key).await
        })
        .await
        .ok()?;
    let threshold = attestor_primitives::calculate_threshold(target_sample_size) as usize;

    let votes: Vec<Vote> = stashed
        .votes
        .iter()
        .filter(|v| shared.bls_store.pubkey(v.attestor.account_id()).is_some())
        .cloned()
        .collect();

    if votes.len() < threshold {
        tracing::warn!(
            digest = ?stashed.digest,
            height = stashed.height,
            needed = threshold,
            got = votes.len(),
            "🗳️ stashed quorum below live threshold"
        );
        return None;
    }

    let mut signed = stashed.signed;
    if votes.len() < stashed.votes.len() {
        // Some signers went inactive — rebuild the aggregate from the surviving votes.
        let sigs = votes.iter().map(|v| v.signature_bls.0).collect::<Vec<_>>();
        let agg_sig = bls_signatures::aggregate(&sigs)
            .map_err(|err| tracing::warn!(?err, "bls re-aggregation of stashed quorum failed"))
            .ok()?;
        let agg_bytes = agg_sig.as_bytes();
        signed.signature = agg_bytes[..].first_chunk::<96>().copied()?;
        signed.attestors = votes.iter().map(|v| v.attestor.clone()).collect();
    }

    Some(Aggregated {
        height: stashed.height,
        digest: stashed.digest,
        attestation_signed: signed,
        votes,
    })
}

// ---------------------------- [ Aggregate + validate (submitter) ] ---------------------------- //

struct Aggregated {
    height: Height,
    digest: Digest,
    attestation_signed: common::types::AttestationSigned,
    votes: Vec<Vote>,
}

enum ValidationError {
    Invalid,
    NoLocalProof,
    /// Live `target_sample_size` is higher than our quorum size — submitting now would hit
    /// `MajorityNotReached` at runtime level. Bail before signing so we never burn fees/turns
    /// on an extrinsic the chain is guaranteed to reject; let the pool keep collecting until
    /// enough peer votes gossip in. Carries the live `target_sample_size` so the handler can
    /// raise the pool's quorum target to match (otherwise the pool keeps re-yielding the same
    /// sub-threshold fork).
    InsufficientVotes {
        needed: usize,
        got: usize,
        target_sample_size: u32,
    },
    External(Error),
}

async fn aggregate_and_validate(
    shared: &Arc<Shared>,
    quorum: &Quorum,
    digest_local: Option<Digest>,
) -> Result<Aggregated, ValidationError> {
    let height = quorum.height;
    let digest = quorum.digest;
    let chain_key = quorum.chain_key;

    // Pre-checks against runtime — each wrapped in the retry shim so transient WS blips don't
    // bubble up as task-fatal `ValidationError::External`s.
    let supported = crate::retry::with_retries(&shared.cc3, &shared.token, |cc3| async move {
        let r = cc3.api().runtime_api().at_latest().await?;
        r.call(
            cc_client::Client::runtime_api()
                .supported_chains_api()
                .is_chain_supported(chain_key),
        )
        .await
        .map_err(cc_client::Error::from)
    })
    .await
    .map_err(|e| ValidationError::External(Error::Rpc(e)))?;
    if !supported {
        return Err(ValidationError::Invalid);
    }

    let is_dup = crate::retry::with_retries(&shared.cc3, &shared.token, |cc3| async move {
        let r = cc3.api().runtime_api().at_latest().await?;
        r.call(
            cc_client::Client::runtime_api()
                .attestor_api()
                .contains_digest(chain_key, cc_client::H256(digest.0), height),
        )
        .await
        .map_err(cc_client::Error::from)
    })
    .await
    .map_err(|e| ValidationError::External(Error::Rpc(e)))?;
    if is_dup {
        return Err(ValidationError::Invalid);
    }

    // Eligibility gate: drop votes whose signer has left the active attestor set since the vote
    // was pooled (election / chill / kick). The runtime filters the submitted `attestors` list
    // to the live `ActiveAttestors`; an aggregate containing an inactive signer would fail its
    // BLS check (`InvalidBlsSignature`) because the runtime-derived aggregate pubkey no longer
    // matches. `bls_store` is refreshed by production on every set change, so it reflects the
    // current membership.
    let votes_active: Vec<Vote> = quorum
        .votes
        .iter()
        .filter(|v| shared.bls_store.pubkey(v.attestor.account_id()).is_some())
        .cloned()
        .collect();
    if votes_active.len() < quorum.votes.len() {
        tracing::info!(
            ?digest,
            height,
            dropped = quorum.votes.len() - votes_active.len(),
            "🍂 dropped votes from signers no longer in the active set"
        );
    }

    // Threshold gate: refresh `target_sample_size` from the chain and refuse to submit if our
    // quorum is now under-threshold. The pool's quorum count is captured at startup; if the
    // active `target_sample_size` grew (epoch rotation, runtime upgrade), an under-threshold
    // submission would be rejected at runtime level (`MajorityNotReached`). Catch it here
    // instead — fees are saved and the pool collects more votes from gossip before the next
    // emission.
    let target_sample_size =
        crate::retry::with_retries(&shared.cc3, &shared.token, |cc3| async move {
            cc3.target_sample_size(chain_key).await
        })
        .await
        .map_err(|e| ValidationError::External(Error::Rpc(e)))?;
    let threshold = attestor_primitives::calculate_threshold(target_sample_size) as usize;
    if votes_active.len() < threshold {
        return Err(ValidationError::InsufficientVotes {
            needed: threshold,
            got: votes_active.len(),
            target_sample_size,
        });
    }

    // Local proof lookup.
    let cached = shared
        .proof_cache
        .get(height, digest)
        .ok_or(ValidationError::NoLocalProof)?;

    // Head/tail/continuity validation (submitter only). `last_finalized` stays an `Option`
    // (height + digest) end to end: collapsing "no finalized attestation yet" into a zero
    // digest would reintroduce the sentinel ambiguity the runtime's own Option-based handling
    // avoids, and the height is needed to mirror the runtime's header-number continuity checks.
    let last_finalized: Option<(Height, cc_client::H256)> =
        crate::retry::with_retries(&shared.cc3, &shared.token, |cc3| async move {
            cc3.fetch_last_finalized(chain_key).await
        })
        .await
        .map_err(|e| ValidationError::External(Error::Rpc(e)))?
        .map(|(h, d)| (h, cc_client::H256(d.0)));

    validate_proof_chain(
        &cached,
        height,
        last_finalized,
        digest_local,
        shared.genesis,
    )
    .map_err(|_| ValidationError::Invalid)?;

    // BLS aggregate.
    let mut rng = rand::rngs::StdRng::from_os_rng();
    let mut votes = votes_active;
    votes.shuffle(&mut rng);

    let sigs = votes.iter().map(|v| v.signature_bls.0).collect::<Vec<_>>();
    let agg_sig =
        bls_signatures::aggregate(&sigs).map_err(|e| ValidationError::External(Error::Bls(e)))?;
    let agg_bytes = agg_sig.as_bytes();
    let bls_aggregate: [u8; 96] = agg_bytes[..]
        .first_chunk::<96>()
        .copied()
        .ok_or(ValidationError::Invalid)?;

    let attestors = votes.iter().map(|v| v.attestor.clone()).collect();
    let signed = attestor_primitives::SignedAttestation {
        attestation: cached.attestation_data,
        signature: bls_aggregate,
        attestors,
        continuity_proof: cached.continuity_proof,
    };

    Ok(Aggregated {
        height,
        digest,
        attestation_signed: signed,
        votes,
    })
}

/// Local mirror of the runtime's `validate_attestation_continuity`, run by the submitter before
/// burning fees on an extrinsic. Kept deliberately aligned with the runtime checks (digest AND
/// header-number continuity) — anything accepted here but rejected at runtime level costs a
/// wasted submission and a quorum re-formation round.
fn validate_proof_chain(
    cached: &crate::proof_cache::CachedProof,
    height: Height,
    last_finalized: Option<(Height, cc_client::H256)>,
    digest_local: Option<Digest>,
    genesis: Height,
) -> Result<(), ()> {
    let proof = &cached.continuity_proof;
    let data = &cached.attestation_data;

    if proof.is_empty() {
        // Only the true chain genesis is exempt from the direct-link check. Keying this on the
        // local start height would let a restarted node (start = last finalized + 1) locally
        // accept a malformed non-genesis empty-proof attestation that the runtime then rejects.
        if height == genesis {
            return Ok(());
        }
        // Direct-link path: runtime accepts an empty continuity proof when prev_digest matches
        // the latest finalized digest AND the height directly follows it (see
        // validate_attestation_continuity's header-number check). Requires a known finalized
        // attestation — `None` means the chain has nothing to link to and only genesis passes.
        let Some((last_height, last_digest)) = last_finalized else {
            return Err(());
        };
        if data.prev_digest().map(|d| cc_client::H256(d.0)) == Some(last_digest)
            && height == last_height.saturating_add(1)
        {
            return Ok(());
        }
        return Err(());
    }

    let start = proof.start_block_number(height);
    let head = proof.compute_continuity_digest(start);
    if Some(head) != data.prev_digest() {
        return Err(());
    }

    if let Some(tail) = proof.tail_prev_digest() {
        let tail_h256 = cc_client::H256(tail.0);
        match last_finalized {
            // Tail links to the finalized attestation/checkpoint: mirror the runtime's tail
            // header-number check — the proof's first block must directly follow it.
            Some((last_height, last_digest)) if tail_h256 == last_digest => {
                if start != last_height.saturating_add(1) {
                    return Err(());
                }
            }
            // Otherwise the tail must link to our own previously-validated digest (pipeline
            // case: the prior submission is still in flight / not yet observed finalized).
            _ => {
                if digest_local.map(|d| cc_client::H256(d.0)) != Some(tail_h256) {
                    return Err(());
                }
            }
        }
    }
    Ok(())
}

// ------------------------------------- [ Submission task ] ------------------------------------ //

struct Submission {
    height: Height,
    outcome: Outcome,
}

enum Outcome {
    /// We submitted. `result` is what the runtime returned. `votes` is the vote set we
    /// submitted with — kept so we can re-inject them on `MajorityNotReached`.
    Eligible {
        result: Result<subxt::blocks::ExtrinsicEvents<subxt::SubstrateConfig>, subxt::Error>,
        votes: Vec<Vote>,
    },
    /// The height was *observed finalized* on chain (via the shared finalized watch) — the
    /// pipeline can safely move on, whatever happened to our own extrinsic.
    Finalized,
    /// We did not submit (chilled, txpool rejection, failed rpc) or stopped watching before an
    /// outcome was known (watch timeout). Finalization was NOT observed. The handler unlocks
    /// the height if it still hasn't finalized after the bounded wait, so future votes and
    /// quorums aren't rejected forever behind a stale validation lock.
    Unresolved,
}

fn spawn_submission(shared: Arc<Shared>, agg: Aggregated) -> tokio::task::JoinHandle<Submission> {
    tokio::spawn(async move {
        let height = agg.height;
        // Stash a copy of the votes — submit_one moves `agg`, but we need them in `Outcome`
        // so MajorityNotReached recovery can re-inject.
        let votes = agg.votes.clone();
        let outcome = match submit_one(&shared, agg).await {
            OutcomeInternal::Eligible(result) => Outcome::Eligible { result, votes },
            OutcomeInternal::Finalized => Outcome::Finalized,
            OutcomeInternal::Unresolved => Outcome::Unresolved,
        };
        Submission { height, outcome }
    })
}

/// The submit_one helper's local return type — same variants as `Outcome` minus the `votes`
/// field, which is added at the spawn site.
enum OutcomeInternal {
    Eligible(Result<subxt::blocks::ExtrinsicEvents<subxt::SubstrateConfig>, subxt::Error>),
    Finalized,
    Unresolved,
}

async fn submit_one(shared: &Arc<Shared>, agg: Aggregated) -> OutcomeInternal {
    let height = agg.height;

    if !*shared.can_attest_rx.borrow() {
        // Deliberate skip, but nothing was observed finalized — Unresolved so the handler
        // unlocks the height and the pool keeps collecting for it (matters if we reactivate).
        tracing::info!(height, "⏳ chilled, skipping submission");
        return OutcomeInternal::Unresolved;
    }

    // Submit jitter (skipped for genesis). Every attestor submits on quorum — the
    // `PrevalidateAttestationCommit` runtime extension dedups the race at txpool admission and
    // evicts losers fee-free (they never reach dispatch), so there is no leader election to do
    // here. The jitter just spreads the otherwise-simultaneous burst of pool admissions + gossip
    // across a couple of seconds; we still bail early if the height finalizes while we wait.
    if height != shared.genesis {
        const SUBMIT_JITTER_MS: u64 = 3_000;
        let delay = std::time::Duration::from_millis(rand::random::<u64>() % SUBMIT_JITTER_MS);
        tracing::debug!(height, ?delay, "🎲 submit jitter");

        let mut rx = shared.latest_finalized_rx.clone();
        let deadline = tokio::time::Instant::now() + delay;
        loop {
            if rx
                .borrow()
                .map(|info| info.height >= height)
                .unwrap_or(false)
            {
                return OutcomeInternal::Finalized;
            }
            tokio::select! {
                // Shutdown / closed finalized watch: the height was never observed finalized,
                // so report Unresolved — Finalized here would skip the handler's unlock path
                // and leave the pool's validation lock set for a height that never landed.
                _ = shared.token.cancelled() => return OutcomeInternal::Unresolved,
                _ = tokio::time::sleep_until(deadline) => break,
                r = rx.changed() => {
                    if r.is_err() { return OutcomeInternal::Unresolved; }
                }
            }
        }

        if !*shared.can_attest_rx.borrow() {
            // Chilled while jittering — same reasoning as the pre-jitter chill check above.
            return OutcomeInternal::Unresolved;
        }
    }

    // Submit.
    let call = cc_client::cc3::tx()
        .attestation()
        .commit_attestation(agg.attestation_signed.into());
    const POOL_INVALID_TX: i32 = 1010;

    // Bound the submit-retry loop as a whole. Transient RPC errors ride out via reconnect
    // within this window; a *deterministic* rejection (metadata drift, a proxy refusing the
    // method — anything that recurs after a successful reconnect) would otherwise loop
    // submit → reconnect-succeeds → resubmit forever, pinning validation: `in_flight` never
    // resolves, every new quorum is stashed via `mark_for_later` and never drained, and the
    // watchdog reads healthy the whole time because each round's reconnect succeeds. Bailing
    // `Unresolved` unlocks the height so a later quorum can retry (or a peer's submission
    // finalizes it externally).
    let submit_deadline = tokio::time::Instant::now() + common::constants::ATTESTATION_TIMEOUT;

    let submit_handle = loop {
        match shared
            .cc3
            .api()
            .tx()
            .sign_and_submit_then_watch_default(&call, shared.signer.keypair())
            .await
        {
            Ok(h) => break h,
            Err(err) => {
                if let subxt::Error::Rpc(subxt::error::RpcError::ClientError(boxed)) = &err {
                    if let Some(subxt::ext::jsonrpsee::core::client::Error::Call(obj)) =
                        boxed.downcast_ref::<subxt::ext::jsonrpsee::core::client::Error>()
                    {
                        if obj.code() == POOL_INVALID_TX {
                            // Substrate maps every txpool admission failure to JSON-RPC 1010 —
                            // both the benign "another attestor won, your duplicate is invalid"
                            // case and the dangerous "your signed extrinsic is structurally
                            // bad" case (BadProof / BadSignature, typically from a stale
                            // mortality anchor or runtime-version mismatch). The two demand
                            // opposite reactions: skip-and-move-on vs. force-reconnect-so-the-
                            // next-tx-uses-fresh-state. Disambiguate via the `data` field.
                            let detail = obj
                                .data()
                                .map(|raw| raw.get().trim_matches('"').to_owned())
                                .unwrap_or_default();
                            let detail_lc = detail.to_ascii_lowercase();
                            if detail_lc.contains("bad signature")
                                || detail_lc.contains("badproof")
                                || detail_lc.contains("bad proof")
                            {
                                tracing::warn!(
                                    height,
                                    code = obj.code(),
                                    %detail,
                                    "🩺 txpool flagged bad-sig — forcing reconnect"
                                );
                                let _ = reconnect(shared).await;
                                return OutcomeInternal::Unresolved;
                            }
                            // Usually "another attestor won the race" — but that competing tx
                            // hasn't finalized yet, so classify as Unresolved: if it does land,
                            // the handler's bounded wait observes it; if not, the height is
                            // unlocked for a retry instead of staying locked forever.
                            tracing::info!(
                                height,
                                code = obj.code(),
                                %detail,
                                "🚫 prevalidation rejected"
                            );
                            return OutcomeInternal::Unresolved;
                        }
                    }
                }
                tracing::warn!(height, ?err, "submission rpc error");
                if reconnect(shared).await.is_err() {
                    return OutcomeInternal::Unresolved;
                }
                // Cancellation during the retry loop: report Unresolved (mirrors the jitter
                // path) rather than spinning through shutdown.
                if shared.token.is_cancelled() {
                    return OutcomeInternal::Unresolved;
                }
                if tokio::time::Instant::now() >= submit_deadline {
                    tracing::warn!(
                        height,
                        "⏰ submit retry window exhausted without txpool acceptance — \
                         releasing the pipeline (height unlocked for a later quorum)"
                    );
                    return OutcomeInternal::Unresolved;
                }
            }
        }
    };

    // Race three signals so the pipeline can never be pinned by a hung submission watch:
    //   1. `wait_for_finalized_success()` — preferred-detail channel (returns the dispatch
    //      result so we can distinguish "we won" / "AttestationExists" / "MajorityNotReached"
    //      / etc.).
    //   2. cc3-stream observation of `BlockAttested(>=height)` via the shared finalized watch
    //      — the height landed externally; our own tx outcome is unknown, but we don't need
    //      to know to make pipeline progress.
    //   3. `ATTESTATION_TIMEOUT` backstop — guards the case where both upstream signals are
    //      stuck.
    // Note: if either of the non-`watch` arms wins, `submit_handle.wait_for_finalized_success()`
    // is dropped silently — but the *extrinsic itself* may still finalize on chain because the
    // node's txpool retains it independently of the watch. The nonce burns either way. The next
    // height's submission can race a still-in-mempool tx and surface as a `Future` (nonce gap)
    // or duplicate-nonce rejection on the next round; both paths self-recover via the
    // `bad_signature`/reconnect arm in `submit_one`. Logged at WARN so the pattern is visible.
    let watch = submit_handle.wait_for_finalized_success();
    tokio::select! {
        result = watch => OutcomeInternal::Eligible(result),
        () = await_block_attested(shared, height) => {
            tracing::warn!(
                height,
                "📡 height observed finalized externally — releasing watch (our submitted tx may still land; nonce burns either way)"
            );
            OutcomeInternal::Finalized
        }
        () = tokio::time::sleep(common::constants::ATTESTATION_TIMEOUT) => {
            tracing::warn!(
                height,
                "🏃 submit watch timed out — unblocking pipeline (extrinsic may still finalize asynchronously; nonce burns either way)"
            );
            OutcomeInternal::Unresolved
        }
    }
}

/// Wait until the cc3 finalized-watch channel reports `info.height >= height`.
async fn await_block_attested(shared: &Arc<Shared>, height: Height) {
    let mut rx = shared.latest_finalized_rx.clone();
    if rx
        .borrow()
        .map(|info| info.height >= height)
        .unwrap_or(false)
    {
        return;
    }
    loop {
        tokio::select! {
            _ = shared.token.cancelled() => return,
            r = rx.changed() => {
                if r.is_err() { return; }
                if rx.borrow().map(|info| info.height >= height).unwrap_or(false) {
                    return;
                }
            }
        }
    }
}

/// Race a single `cc3.reconnect()` attempt against the shutdown token.
///
/// `Client::reconnect()` already owns the shared backoff (`Mutex<BackoffState>`) and a per-dial
/// timeout, so we don't add another retry layer here — a previous version wrapped this in
/// `tokio_retry::Retry`, which double-coordinated backoff and contradicted the unbounded-retry
/// design in `retry.rs` / the cc3 stream. Callers that need ride-out behaviour should call this
/// from inside `with_retries`; ad-hoc submission paths (which already loop on transient errors)
/// just want one attempt + cancellation.
async fn reconnect(shared: &Arc<Shared>) -> Result<(), ()> {
    tokio::select! {
        _ = shared.token.cancelled() => Err(()),
        r = shared.cc3.reconnect() => r.map_err(|err| {
            tracing::warn!(?err, "cc3 reconnect attempt failed");
        }),
    }
}

async fn wait_finalized(shared: &Arc<Shared>, height: Height) {
    let mut rx = shared.latest_finalized_rx.clone();
    if rx
        .borrow()
        .map(|info| info.height >= height)
        .unwrap_or(false)
    {
        return;
    }
    let deadline = tokio::time::Instant::now() + common::constants::ATTESTATION_TIMEOUT;
    loop {
        tokio::select! {
            _ = shared.token.cancelled() => return,
            _ = tokio::time::sleep_until(deadline) => {
                tracing::warn!(height, "🏃 finalization timed out");
                return;
            }
            r = rx.changed() => {
                if r.is_err() { return; }
                if rx.borrow().map(|info| info.height >= height).unwrap_or(false) {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use attestor_primitives::Digest;

    use super::validate_proof_chain;
    use crate::proof_cache::CachedProof;

    fn cached(prev: Option<Digest>) -> CachedProof {
        CachedProof {
            attestation_data: attestor_primitives::AttestationData::new(
                1u64,
                2,
                Digest::from([0xAA; 32]),
                sp_core::H256::from([0xBB; 32]),
                prev,
            ),
            continuity_proof: Default::default(),
        }
    }

    #[test]
    fn empty_proof_allowed_at_genesis() {
        let cached = cached(None);
        assert!(validate_proof_chain(&cached, 0, None, None, 0).is_ok());
    }

    #[test]
    fn empty_proof_allowed_on_direct_link_to_last_finalized() {
        let prev = Digest::from([0x11; 32]);
        let last_finalized = Some((1, cc_client::H256(prev.0)));
        let cached = cached(Some(prev));
        assert!(validate_proof_chain(&cached, 2, last_finalized, None, 0).is_ok());
    }

    #[test]
    fn empty_proof_rejected_when_prev_digest_does_not_match_last_finalized() {
        let cached = cached(Some(Digest::from([0x11; 32])));
        let last_finalized = Some((1, cc_client::H256::from([0x22; 32])));
        assert!(validate_proof_chain(&cached, 2, last_finalized, None, 0).is_err());
    }

    #[test]
    fn empty_proof_rejected_at_non_genesis_without_prev_digest() {
        let cached = cached(None);
        assert!(validate_proof_chain(&cached, 2, None, None, 0).is_err());
    }

    /// Runtime enforces `height == prev_header_number + 1` on the direct-link path; the local
    /// check must too — a matching digest at a non-adjacent height is still rejected on-chain.
    #[test]
    fn empty_proof_rejected_when_height_does_not_directly_follow_last_finalized() {
        let prev = Digest::from([0x11; 32]);
        let last_finalized = Some((1, cc_client::H256(prev.0)));
        let cached = cached(Some(prev));
        assert!(validate_proof_chain(&cached, 3, last_finalized, None, 0).is_err());
    }

    /// Non-genesis attestation with an empty proof cannot link to anything when the chain has no
    /// finalized attestation — the zero-digest sentinel must not sneak it through.
    #[test]
    fn empty_proof_rejected_at_non_genesis_when_nothing_finalized() {
        let prev = Digest::from([0x11; 32]);
        let cached = cached(Some(prev));
        assert!(validate_proof_chain(&cached, 2, None, None, 0).is_err());
    }

    fn cached_with_proof(height: attestor_primitives::Height, tail: Digest) -> CachedProof {
        // One-root proof covering block `height - 1`, chaining from `tail`.
        let root = sp_core::H256::from([0xCC; 32]);
        let proof =
            attestor_primitives::block::ContinuityProof::new(sp_core::H256(tail.0), vec![root]);
        let head = proof.compute_continuity_digest(height - 1);
        CachedProof {
            attestation_data: attestor_primitives::AttestationData::new(
                1u64,
                height,
                Digest::from([0xAA; 32]),
                sp_core::H256::from([0xBB; 32]),
                Some(Digest::from(head.0)),
            ),
            continuity_proof: proof,
        }
    }

    /// Tail links to the finalized digest at the right height (proof starts directly after it).
    #[test]
    fn proof_tail_accepted_when_it_directly_follows_last_finalized() {
        let tail = Digest::from([0x11; 32]);
        let cached = cached_with_proof(10, tail);
        // Proof covers block 9, so the finalized attestation must sit at height 8.
        let last_finalized = Some((8, cc_client::H256(tail.0)));
        assert!(validate_proof_chain(&cached, 10, last_finalized, None, 0).is_ok());
    }

    /// Runtime verifies the tail's expected header number; a tail digest matching the finalized
    /// digest at the wrong height must be rejected locally too.
    #[test]
    fn proof_tail_rejected_when_finalized_height_does_not_match() {
        let tail = Digest::from([0x11; 32]);
        let cached = cached_with_proof(10, tail);
        // Finalized attestation claims height 5, but the proof starts at block 9.
        let last_finalized = Some((5, cc_client::H256(tail.0)));
        assert!(validate_proof_chain(&cached, 10, last_finalized, None, 0).is_err());
    }

    /// Pipeline case: the tail may link to our previously-validated (not yet finalized) digest.
    #[test]
    fn proof_tail_accepted_when_it_links_to_local_digest() {
        let tail = Digest::from([0x33; 32]);
        let cached = cached_with_proof(10, tail);
        let last_finalized = Some((5, cc_client::H256::from([0x22; 32])));
        assert!(validate_proof_chain(&cached, 10, last_finalized, Some(tail), 0).is_ok());
    }
}
