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

            // New quorum from the pool.
            Some((quorum, permit)) = pool_rx.recv() => {
                // If we're already submitting at this height, drop the quorum on the floor —
                // the pool will keep us informed, and re-stashing the same height is wasted
                // work.
                if Some(quorum.height) == in_flight_height {
                    tracing::debug!(
                        height = quorum.height,
                        digest = ?quorum.digest,
                        "🪞 duplicate-height quorum while submitting — discarding"
                    );
                    pool_rx.mark_valid(permit);
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
            // someone's continuity proof. Drop the fork; we'll keep listening for the
            // matching-digest quorum we're producing locally.
            tracing::warn!(
                ?digest,
                height,
                "🙈 quorum digest has no matching local proof — skipping submission"
            );
            pool_rx.mark_valid(permit);
            return Ok(());
        }
        Err(ValidationError::InsufficientVotes { needed, got }) => {
            // Live threshold is higher than our quorum size. Unlock the height in the pool
            // (so new gossiped votes can re-trigger a quorum at the bigger threshold) and
            // re-inject our existing votes so they're not lost. We never submitted, so no
            // fees were burned and no on-chain race was created.
            tracing::warn!(
                ?digest,
                height,
                needed,
                got,
                "🗳️ insufficient votes for current threshold — re-injecting"
            );
            shared.pool_send.note_majority_not_reached(height);
            for vote in quorum.votes {
                let digest = vote.digest();
                match shared.pool_send.send(vote) {
                    Some(Ok(())) | None => {}
                    Some(Err(err)) => err.log_error(digest),
                }
            }
            pool_rx.mark_valid(permit);
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
                tracing::warn!(height, ?err, "⛔ runtime rejected submission");
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
            tracing::warn!(height, ?err, "⛔ submission rpc error");
        }
        Outcome::NotEligible => {
            tracing::info!(height, "🚦 not selected, deferring to other attestors");
        }
        Outcome::Finalized => {
            tracing::info!(height, "✅ finalized externally");
        }
    }

    // Wait briefly for this height to finalize on chain before pulling the next stash.
    wait_finalized(shared, height).await;

    if let Some(stashed) = pool_rx.take_next_validated() {
        tracing::info!(
            digest = ?stashed.digest, height = stashed.height,
            "🛫 submitting pre-validated stash"
        );
        *in_flight_height = Some(stashed.height);
        *in_flight = Some(spawn_submission(
            shared.clone(),
            Aggregated {
                height: stashed.height,
                digest: stashed.digest,
                attestation_signed: stashed.signed,
                votes: stashed.votes,
            },
        ));
    }
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
    /// enough peer votes gossip in.
    InsufficientVotes {
        needed: usize,
        got: usize,
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
    if quorum.votes.len() < threshold {
        return Err(ValidationError::InsufficientVotes {
            needed: threshold,
            got: quorum.votes.len(),
        });
    }

    // Local proof lookup.
    let cached = shared
        .proof_cache
        .get(height, digest)
        .ok_or(ValidationError::NoLocalProof)?;

    // Head/tail/continuity validation (submitter only).
    let last_finalized = crate::retry::with_retries(&shared.cc3, &shared.token, |cc3| async move {
        let r = cc3.api().runtime_api().at_latest().await?;
        r.call(
            cc_client::Client::runtime_api()
                .attestor_api()
                .last_digest(chain_key),
        )
        .await
        .map_err(cc_client::Error::from)
    })
    .await
    .map_err(|e| ValidationError::External(Error::Rpc(e)))?
    .unwrap_or_else(cc_client::H256::zero);

    validate_proof_chain(
        &cached,
        height,
        last_finalized,
        digest_local,
        shared.start_height,
    )
    .map_err(|_| ValidationError::Invalid)?;

    // BLS aggregate.
    let mut rng = rand::rngs::StdRng::from_os_rng();
    let mut votes = quorum.votes.clone();
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

fn validate_proof_chain(
    cached: &crate::proof_cache::CachedProof,
    height: Height,
    last_finalized: cc_client::H256,
    digest_local: Option<Digest>,
    start_height: Height,
) -> Result<(), ()> {
    let proof = &cached.continuity_proof;
    let data = &cached.attestation_data;

    if proof.is_empty() {
        if height == start_height {
            return Ok(());
        }
        // Direct-link path: runtime accepts empty continuity proof when prev_digest
        // matches the latest finalized digest (see validate_attestation_continuity).
        if data.prev_digest().map(|d| cc_client::H256(d.0)) == Some(last_finalized) {
            return Ok(());
        }
        return Err(());
    }

    if !proof.is_empty() {
        let start = proof.start_block_number(height);
        let head = proof.compute_continuity_digest(start);
        if Some(head) != data.prev_digest() {
            return Err(());
        }
    }

    if let Some(tail) = proof.tail_prev_digest() {
        let tail_h256 = cc_client::H256(tail.0);
        if tail_h256 != last_finalized
            && digest_local.map(|d| cc_client::H256(d.0)) != Some(tail_h256)
        {
            return Err(());
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
    NotEligible,
    Finalized,
}

fn spawn_submission(shared: Arc<Shared>, agg: Aggregated) -> tokio::task::JoinHandle<Submission> {
    tokio::spawn(async move {
        let height = agg.height;
        // Stash a copy of the votes — submit_one moves `agg`, but we need them in `Outcome`
        // so MajorityNotReached recovery can re-inject.
        let votes = agg.votes.clone();
        let outcome = match submit_one(&shared, agg).await {
            OutcomeInternal::Eligible(result) => Outcome::Eligible { result, votes },
            OutcomeInternal::NotEligible => Outcome::NotEligible,
            OutcomeInternal::Finalized => Outcome::Finalized,
        };
        Submission { height, outcome }
    })
}

/// The submit_one helper's local return type — same variants as `Outcome` minus the `votes`
/// field, which is added at the spawn site.
enum OutcomeInternal {
    Eligible(Result<subxt::blocks::ExtrinsicEvents<subxt::SubstrateConfig>, subxt::Error>),
    NotEligible,
    Finalized,
}

async fn submit_one(shared: &Arc<Shared>, agg: Aggregated) -> OutcomeInternal {
    let height = agg.height;
    let chain_key = agg.attestation_signed.attestation.chain_key;

    if !*shared.can_attest_rx.borrow() {
        tracing::info!(height, "⏳ chilled, skipping submission");
        return OutcomeInternal::Finalized;
    }

    let (randomness, epoch_index) = match shared.cc3.fetch_babe_randomness_two_epoch_ago().await {
        Ok(r) => r,
        Err(err) => {
            tracing::warn!(?err, "cc3 randomness failed");
            let _ = reconnect(shared).await;
            return OutcomeInternal::NotEligible;
        }
    };

    // Rank-backoff (skipped for genesis).
    if height != shared.genesis {
        let vrf = match shared
            .cc3
            .sign_vrf_submission(chain_key, height, randomness, epoch_index)
            .await
        {
            Ok(v) => v,
            Err(cc_client::Error::FailedToCreateProofOfInclusion(_)) => {
                tracing::info!(height, "🚦 not selected");
                return OutcomeInternal::NotEligible;
            }
            Err(err) => {
                tracing::warn!(?err, "vrf err");
                let _ = reconnect(shared).await;
                return OutcomeInternal::NotEligible;
            }
        };

        let mut rank_input = Vec::with_capacity(vrf.output.len() + 8);
        rank_input.extend_from_slice(&vrf.output);
        rank_input.extend_from_slice(&height.to_be_bytes());
        let rank_hash = sp_io::hashing::keccak_256(&rank_input);
        const RANKS: u64 = 8;
        let rank = u64::from_be_bytes([
            rank_hash[0],
            rank_hash[1],
            rank_hash[2],
            rank_hash[3],
            rank_hash[4],
            rank_hash[5],
            rank_hash[6],
            rank_hash[7],
        ]) % RANKS;
        const SUBMIT_FIN_DELAY: u64 = 17;
        let delay = std::time::Duration::from_secs(rank * SUBMIT_FIN_DELAY);
        tracing::info!(height, rank, ?delay, "🏁 rank backoff");

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
                _ = shared.token.cancelled() => return OutcomeInternal::Finalized,
                _ = tokio::time::sleep_until(deadline) => break,
                r = rx.changed() => {
                    if r.is_err() { return OutcomeInternal::Finalized; }
                }
            }
        }

        if !*shared.can_attest_rx.borrow() {
            return OutcomeInternal::Finalized;
        }
    }

    // Submit.
    let call = cc_client::cc3::tx()
        .attestation()
        .commit_attestation(agg.attestation_signed.into());
    const POOL_INVALID_TX: i32 = 1010;

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
                                return OutcomeInternal::Finalized;
                            }
                            tracing::info!(
                                height,
                                code = obj.code(),
                                %detail,
                                "🚫 prevalidation rejected"
                            );
                            return OutcomeInternal::Finalized;
                        }
                    }
                }
                tracing::warn!(height, ?err, "submission rpc error");
                if reconnect(shared).await.is_err() {
                    return OutcomeInternal::Finalized;
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
    let watch = submit_handle.wait_for_finalized_success();
    tokio::select! {
        result = watch => OutcomeInternal::Eligible(result),
        () = await_block_attested(shared, height) => {
            tracing::info!(height, "📡 height observed finalized externally — releasing watch");
            OutcomeInternal::Finalized
        }
        () = tokio::time::sleep(common::constants::ATTESTATION_TIMEOUT) => {
            tracing::warn!(height, "🏃 submit watch timed out — unblocking pipeline");
            OutcomeInternal::Finalized
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

async fn reconnect(shared: &Arc<Shared>) -> Result<(), ()> {
    let strategy = tokio_retry::strategy::ExponentialBackoff::from_millis(100)
        .max_delay(std::time::Duration::from_millis(5_000))
        .map(tokio_retry::strategy::jitter);
    let cc3 = shared.cc3.clone();
    let retry = tokio_retry::Retry::spawn(strategy, move || {
        let cc3 = cc3.clone();
        async move { cc3.reconnect().await }
    });
    tokio::select! {
        _ = shared.token.cancelled() => Err(()),
        r = retry => match r {
            Ok(()) => Ok(()),
            Err(err) => {
                tracing::error!(?err, "reconnect failed permanently");
                Err(())
            }
        }
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
        assert!(validate_proof_chain(&cached, 0, cc_client::H256::zero(), None, 0).is_ok());
    }

    #[test]
    fn empty_proof_allowed_on_direct_link_to_last_finalized() {
        let prev = Digest::from([0x11; 32]);
        let last_finalized = cc_client::H256(prev.0);
        let cached = cached(Some(prev));
        assert!(validate_proof_chain(&cached, 2, last_finalized, None, 0).is_ok());
    }

    #[test]
    fn empty_proof_rejected_when_prev_digest_does_not_match_last_finalized() {
        let cached = cached(Some(Digest::from([0x11; 32])));
        assert!(
            validate_proof_chain(&cached, 2, cc_client::H256::from([0x22; 32]), None, 0,).is_err()
        );
    }

    #[test]
    fn empty_proof_rejected_at_non_genesis_without_prev_digest() {
        let cached = cached(None);
        assert!(validate_proof_chain(&cached, 2, cc_client::H256::zero(), None, 0).is_err());
    }
}
