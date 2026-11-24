//! Production task — the chain state machine.
//!
//! **Sole owner** of:
//!   - the CC3 finality event subscription (one, not three),
//!   - the eth `StreamAttestation`,
//!   - writes to `shared.can_attest_tx`, `shared.latest_finalized_tx`, `shared.proof_cache`,
//!     and `shared.pool_send.note_*` chain-event hooks.
//!
//! On each freshly produced eth attestation it:
//!   1. caches `(attestation_data, continuity_proof)` in `shared.proof_cache`,
//!   2. signs a lightweight `Vote { chain_key, height, digest, attestor, sig_bls }`,
//!   3. ships the vote into the local pool **and** into the gossip channel.
//!
//! Genesis is just the first production round — no out-of-band code path.

use std::num::NonZero;
use std::sync::Arc;

use futures::{StreamExt as _, TryStreamExt as _};

use cc_client::attestation::CcEvent;

use crate::error::Error;
use crate::shared::{AttestationInfo, Shared};

pub async fn run(
    shared: Arc<Shared>,
    start_attestation: Option<AttestationInfo>,
) -> Result<(), Error> {
    let interval = shared.attestation_interval();
    let start_height = shared.start_height;
    let genesis = shared.genesis;

    // ----------------------------------* eth streams *--------------------------------------- //

    // CPU-bound merkleization parallelism (see `roots::Config::max_parallelism`). Reserve a
    // single core for the rest of the shared tokio runtime — the other tasks are I/O-bound and
    // mostly idle, so one core of headroom is plenty. The old `WORKER_COUNT + 1` reservation
    // was a v1 artifact (v2 spawns no OS worker threads); it saturated to 1 on every ≤4-CPU pod,
    // needlessly single-threading deep root rebuilds after an outage or revert.
    let max_parallelism = std::thread::available_parallelism()
        .ok()
        .and_then(|n| n.get().checked_sub(1))
        .and_then(NonZero::new)
        .unwrap_or(NonZero::<usize>::MIN);

    let roots_cfg = stream::eth::roots::ConfigBuilder::new()
        .with_client(shared.eth.clone())
        .with_start_height(start_height)
        .with_bound(stream::eth::roots::Boundary::Source(shared.maturity))
        .with_max_concurrency(shared.eth_max_concurrency)
        .with_max_parallelism(max_parallelism)
        .with_encoding(shared.encoding)
        .with_head_poll_interval(shared.eth_head_poll_interval)
        .with_stop_on_unrecoverable(true)
        .build();
    let stream_roots = stream::eth::StreamRoots::new(roots_cfg).await;
    // Taken before the stream is boxed; the streams its resets build write to the same slot.
    let roots_ended_by = stream_roots.unrecoverable_slot();

    let tip_cfg = stream::eth::tip::ConfigBuilder::new()
        .with_client(shared.eth.clone())
        .with_maturity(shared.maturity)
        .with_start_height(start_height)
        .with_head_poll_interval(shared.eth_head_poll_interval)
        .build();
    let stream_tip = stream::eth::StreamTip::new(tip_cfg).await;

    use stream::util::ChainExt as _;
    let attestation_cfg = stream::attestation::ConfigBuilder::new()
        .with_signer(shared.signer.clone())
        .with_chain_key(shared.chain_key)
        .with_bls_key(shared.bls_key)
        .with_stream_roots(stream_roots.boxed_data())
        .with_stream_tip(stream_tip.boxed_data())
        .with_attestation_interval(interval)
        .with_attestation_prev(
            start_attestation
                .map(|i| stream::util::AttestationInfo {
                    height: i.height,
                    digest: i.digest,
                })
                .unwrap_or(stream::util::AttestationInfo {
                    height: genesis,
                    ..Default::default()
                }),
        )
        .with_max_catchup(shared.max_catchup())
        .build();
    let mut stream_attestation = stream::attestation::StreamAttestation::new(attestation_cfg);

    // ----------------------------------* cc3 stream *---------------------------------------- //
    // Opened before the (optional) genesis step so we can wait for the genesis BlockAttested
    // event on the same subscription we use in the main loop.

    let cc3_cfg = stream::cc3::ConfigBuilder::new()
        .with_cc3(shared.cc3.clone())
        .with_chain_keys(vec![shared.chain_key])
        .build();
    let mut events = stream::cc3::StreamCC3::new(cc3_cfg)
        .await
        .map_err(Error::Init)?;

    // ----------------------------------* genesis (if needed) *------------------------------- //
    //
    // When start_attestation is None this attestor is bootstrapping the attestation chain. We
    // generate + ship the genesis vote, then wait for cc3 to emit BlockAttested(genesis_height)
    // with the *real* digest. Only after seeing it do we tell `stream_attestation` and the
    // shared finalized watch — otherwise the eth stream would build subsequent continuity
    // proofs with a ZERO tail digest, and submissions would fail with
    // InvalidAttestationContinuityProofTail at every subsequent height.

    let mut latest_cc3 = if let Some(info) = start_attestation {
        info
    } else {
        // The genesis vote is signed from a direct block fetch, outside the maturity-bounded
        // roots stream, so it waits for its own maturity and for eligibility. cc3 events are
        // applied meanwhile, and a genesis finalized by peers ends the wait unsigned. Shutdown
        // exits cleanly like every other task instead of failing startup.
        tracing::info!(genesis, "⏳ waiting for the genesis block to mature");
        let info =
            match wait_for_genesis(&shared, &mut events, &mut stream_attestation, genesis, true)
                .await?
            {
                GenesisWait::Shutdown => {
                    tracing::info!("🔌 shutdown while waiting for genesis block maturity");
                    return Ok(());
                }
                GenesisWait::Attested(info) => {
                    // No local vote was signed during the wait, so the stall axis would read
                    // "chain attests, we don't". Same re-arm as a revert: a full stall window.
                    shared.health.rearm_stall_window();
                    info
                }
                GenesisWait::Mature => {
                    tracing::info!(genesis, "👶 generating genesis attestation");
                    let block = shared
                        .eth
                        .get_block(genesis, shared.encoding)
                        .await
                        .map_err(|e| Error::Init(anyhow::anyhow!("genesis block fetch: {e}")))?;
                    let root_info = stream::util::RootInfo {
                        height: genesis,
                        root: eth::simple_merkle_tree(&block).root(),
                        hash: attestor_primitives::Digest::from(*block.hash()),
                    };
                    let genesis_att = stream_attestation.generate_attestation_genesis(root_info);
                    emit_local(&shared, &genesis_att).await;

                    // Wait for the genesis BlockAttested on cc3 — we need the *real* digest.
                    tracing::info!(genesis, "⏲️ waiting for genesis attestation to finalize");
                    match wait_for_genesis(
                        &shared,
                        &mut events,
                        &mut stream_attestation,
                        genesis,
                        false,
                    )
                    .await?
                    {
                        GenesisWait::Attested(info) => info,
                        GenesisWait::Shutdown => {
                            tracing::info!("🔌 shutdown while waiting for genesis attestation");
                            return Ok(());
                        }
                        // No gate was requested, so it cannot open.
                        GenesisWait::Mature => {
                            return Err(Error::Init(anyhow::anyhow!(
                                "genesis gate opened without being requested"
                            )))
                        }
                    }
                }
            };
        stream_attestation.note_attestation_finalization(stream::util::AttestationInfo {
            height: info.height,
            digest: info.digest,
        });
        // Tell the pool too (mirrors the steady-state `BlockAttested` handler) — otherwise the
        // pool's on-chain finalized floor stays unset and stale genesis-fork votes linger until
        // some later height happens to exceed the genesis cursor.
        shared
            .pool_send
            .note_attestation_finalization(info.height, info.digest);
        let _ = shared.latest_finalized_tx.send(Some(info));
        shared.proof_cache.note_finalized(info.height);
        shared.metrics.set_attestation_finalized(info.height);
        // Watchdog: genesis finalizing is the first on-chain attestation observation.
        shared.health.note_chain_attested();
        info
    };

    // ----------------------------------* main loop *----------------------------------------- //

    // Dedicated receiver for waking the loop on an eligibility toggle. `can_attest` gates the
    // `stream_attestation` arm below, and the flip to `true` now happens asynchronously (the
    // post-election warm-up task) — without a `changed()` arm the loop would keep the arm
    // disabled until some unrelated cc3 batch happened to wake it and re-read the flag. Separate
    // from the `borrow()` read below so its seen-version tracking is independent.
    let mut eligibility_rx = shared.can_attest_rx.clone();

    loop {
        let can_attest = *shared.can_attest_rx.borrow();
        tokio::select! {
            biased;
            _ = shared.token.cancelled() => return Ok(()),

            // Wake on an eligibility toggle so the guarded arm is (dis/re-)enabled promptly.
            // `changed()` only errors once every sender is dropped — impossible while we hold
            // `shared` (which owns `can_attest_tx`) — so a plain wake-and-re-loop is enough.
            _ = eligibility_rx.changed() => {}

            maybe_batch = events.next() => {
                // The cc3 stream only ends on a *permanent* backfill error (state pruned beyond
                // recovery — see `is_permanent_backfill_error` in the stream). Fail fast so the
                // supervisor restarts us: a fresh boot re-seeds from the current head with no
                // backfill, which is the only way to get past pruned state. A `Some(batch) = …`
                // pattern here would instead silently disable this arm and leave production
                // half-alive on the eth side.
                let Some(batch) = maybe_batch else {
                    return Err(Error::Cc3Stream(stream::cc3::Error::EndOfStream));
                };
                tokio::select! {
                    _ = shared.token.cancelled() => return Ok(()),
                    result = handle_cc3_batch(
                        &shared,
                        &mut stream_attestation,
                        &mut latest_cc3,
                        batch,
                    ) => result?,
                }
            }

            maybe_attestation = stream_attestation.next(), if can_attest => {
                // The stream ends on a block no retry can fix, or on an interrupt. Fail fast on the
                // first so the supervisor restarts us; a `Some(..)` pattern would instead silently
                // disable this arm and leave production half-alive.
                let Some(attestation) = maybe_attestation else {
                    if roots_ended_by.interrupted() {
                        // Not a failure: the cancellation token, about to fire, ends the task.
                        shared.token.cancelled().await;
                        return Ok(());
                    }
                    return Err(Error::EthStream(
                        roots_ended_by.take().unwrap_or(stream::eth::Error::StreamEnd),
                    ));
                };
                emit_local(&shared, &attestation).await;
                // Refresh the source-chain lag gauge with the tip the stream is tracking —
                // the config-time seed would otherwise never be updated after startup and the
                // gauge would drift into meaninglessness. Tip is 0 until the tip stream has
                // yielded once (e.g. the genesis bootstrap path); skip rather than report a
                // bogus positive lag.
                let tip = stream_attestation.tip();
                if tip > 0 {
                    shared.metrics.update_attestation_lag_eth(
                        attestation.header_number(),
                        tip,
                        shared.attestation_interval(),
                    );
                }
            }
        }
    }
}

// ------------------------------------ [ Emit local vote ] ------------------------------------- //

/// Cache the proof, sign the lightweight vote, push to pool + gossip.
async fn emit_local(shared: &Arc<Shared>, attestation: &common::types::Attestation) {
    let height = attestation.header_number();
    let digest = attestation.digest();
    let now = std::time::Instant::now();

    tracing::info!(
        ?digest, height,
        attestor = %attestation.attestor,
        "📡 produced local attestation",
    );

    // Watchdog: our source-chain pipeline demonstrably produced (the stall axis' local side).
    shared.health.note_local_attested();

    // Cache the proof + AttestationData so future incoming votes at this height can be verified
    // and so the validation task has the proof to submit later.
    shared.proof_cache.insert(
        attestation.attestation_data.clone(),
        attestation.continuity_proof.clone(),
    );

    // Signal p2p so it can drain any votes it queued for this height pending local data.
    let _ = shared.local_produced_tx.send(Some(height));

    // Build the lightweight vote (no continuity proof on the wire).
    let local_data = crate::vote::LocalAttestationData {
        serialized: attestation.attestation_data.serialize(),
        digest,
    };
    let vote = crate::vote::sign_vote(
        &shared.bls_key,
        shared.attestor_id.clone(),
        shared.chain_key,
        height,
        &local_data,
    );

    // Local pool insertion. This succeeds for our own digest; if the pool rejects it we log and
    // continue.
    if let Some(Err(err)) = shared.pool_send.send(vote.clone()) {
        err.log_error(digest);
    }

    // Send to gossip. The channel is unbounded so this never blocks production and never drops
    // a signed vote (a vote dropped here would be lost to the whole network — p2p's retry queue
    // only covers votes that reached it). The p2p task drains unconditionally, bounding memory
    // by its processing rate. A closed channel means p2p has exited; the supervisor will act on
    // that.
    if shared.gossip_tx.send(vote).is_err() {
        tracing::warn!(?digest, "📭 gossip channel closed");
    }

    // metrics
    shared
        .metrics
        .update_attestation_handler_delay(now.elapsed());
    shared.metrics.set_attestation_local(height);
}

// ----------------------------------- [ Handle CC3 events ] ------------------------------------ //

async fn handle_cc3_batch(
    shared: &Arc<Shared>,
    stream_attestation: &mut stream::attestation::StreamAttestation,
    latest_cc3: &mut AttestationInfo,
    mut batch: stream::cc3::StreamEvents,
) -> Result<(), Error> {
    // Steady liveness pulse: a cc3 finalized batch arriving means the connection is live and the
    // production loop is turning. The watchdog reads this to distinguish "alive" from "wedged".
    shared.health.note_progress();
    while let Some(event) = batch.try_next().await? {
        handle_one(shared, stream_attestation, latest_cc3, event).await?;
    }
    Ok(())
}

/// Apply an eligibility transition (from an election / chill / kick) to `can_attest` and the
/// health stall axis.
///
/// Enabling *from a disabled state* is warmed up: we hold `can_attest` false for
/// [`POST_ELECTION_WARMUP`] before flipping it true, so peers have time to process the same
/// `AttestorsElected` event and refresh their BLS stores. Otherwise our first post-reactivation
/// votes reach peers that can't yet verify them and are rejected — and gossipsub marks rejected
/// messages seen and never redelivers them, so those votes are lost permanently. This mirrors
/// the startup election paths in `startup::wait_for_eligible`.
///
/// Disabling, or re-electing while already enabled (we were never off, so peers already know us),
/// applies immediately. Every call bumps `eligibility_gen`; the warm-up task captures its value
/// and enables only if still current — so a chill/kick landing inside the warm-up window cancels
/// the pending enable instead of racing it. `set_attest_expected` is set immediately in all cases
/// (the stall axis re-arms with a full grace window that dwarfs the 30s warm-up).
/// What an eligibility event should do, decided purely from the previous *target* and the new
/// eligibility. Split out so the transition matrix is unit-testable without a full `Shared`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EligibilityAction {
    /// Target unchanged — do nothing (crucially: no `eligibility_gen` bump, so a pending warm-up
    /// survives unrelated `AttestorKicked` / peer-chill events that re-fire the handler).
    NoOp,
    /// Target went true→false — disable immediately.
    DisableNow,
    /// Target went false→true — warm up before enabling (peers need `POST_ELECTION_WARMUP` to
    /// refresh their BLS stores; un-warmed first votes are dropped and never redelivered).
    WarmUpEnable,
}

fn eligibility_action(prev_target: bool, new_eligible: bool) -> EligibilityAction {
    match (prev_target, new_eligible) {
        (a, b) if a == b => EligibilityAction::NoOp,
        (_, true) => EligibilityAction::WarmUpEnable,
        (_, false) => EligibilityAction::DisableNow,
    }
}

/// Re-read `ActiveAttestors` from the chain and refresh the local committee view (BLS store,
/// pool allow-set, `can_attest`) from it.
///
/// Used for chill/kick and for the once-per-epoch reconcile. `with_retries`, matching the
/// election path: a transient RPC blip must ride out with backoff, not bubble up and terminate
/// the whole production task (the unscoped `staking::Kicked` event makes the chill/kick handler
/// fire more often than actual committee changes). Eligibility is derived from the refreshed
/// on-chain membership rather than from any event: `staking::Kicked` carries no chain scope, so
/// trusting the event could wrongly chill us on an unrelated kick — the authoritative active set
/// can't. The BLS/pool refresh runs before `apply_eligibility` for the reason given at the
/// `AttestorsElected` handler.
async fn reconcile_committee_from_chain(shared: &Arc<Shared>) -> Result<(), Error> {
    let chain_key = shared.chain_key;
    let attestors = crate::retry::with_retries(&shared.cc3, &shared.token, |cc3| async move {
        cc3.get_attestor_active_set(chain_key).await
    })
    .await
    .map_err(Error::Rpc)?;
    let eligible = attestors.contains(&shared.account_id);
    shared
        .bls_store
        .note_attestors_elected(&shared.cc3, &shared.token, &attestors)
        .await
        .map_err(Error::Rpc)?;
    shared.pool_send.note_attestors_elected(attestors);
    // Membership is half of the quorum denominator: an election, chill or kick moves the
    // threshold even with the target unchanged. Refresh before arming eligibility so a node
    // never attests against a stale quorum.
    refresh_quorum(shared).await?;
    apply_eligibility(shared, eligible);
    Ok(())
}

fn apply_eligibility(shared: &Arc<Shared>, eligible: bool) {
    use std::sync::atomic::Ordering;
    let prev_target = shared.attest_target.swap(eligible, Ordering::SeqCst);
    match eligibility_action(prev_target, eligible) {
        EligibilityAction::NoOp => {}
        EligibilityAction::DisableNow => {
            // Bump the generation so any pending warm-up sees a stale value and does not enable.
            shared.eligibility_gen.fetch_add(1, Ordering::SeqCst);
            shared.health.set_attest_expected(false);
            let _ = shared.can_attest_tx.send(false);
        }
        EligibilityAction::WarmUpEnable => {
            let gen = shared
                .eligibility_gen
                .fetch_add(1, Ordering::SeqCst)
                .wrapping_add(1);
            shared.health.set_attest_expected(true);
            tracing::info!(
                "⏳ eligible — warming up before attesting (peers refreshing BLS stores)"
            );
            let shared = shared.clone();
            tokio::spawn(async move {
                tokio::select! {
                    _ = shared.token.cancelled() => {}
                    () = tokio::time::sleep(common::constants::POST_ELECTION_WARMUP) => {
                        if shared.eligibility_gen.load(Ordering::SeqCst) == gen {
                            let _ = shared.can_attest_tx.send(true);
                            tracing::info!("☀️ warm-up complete — attesting");
                        } else {
                            tracing::info!("↩️ warm-up superseded by a later eligibility change — not enabling");
                        }
                    }
                }
            });
        }
    }
}

/// Recompute the quorum from live chain state and push it to the pool and the health gauge.
///
/// The quorum is `2/3+1` of `|ActiveAttestors|`, so it moves with *membership*. Every seam that
/// changes the active set has to call this: election, chill and kick. Missing one leaves the node
/// enforcing a stale threshold — too high and it stops submitting entirely, too low and it burns
/// fees on `MajorityNotReached`.
///
/// `Client::quorum` reads both inputs from a single storage snapshot, so this cannot mix an
/// active-set size from one block with a target from another. It reads at `latest` rather than
/// trusting the event payload, matching the chill/kick path's existing reasoning: the
/// authoritative set is what the chain holds now, not what an event carried.
/// Production is the sole writer of the pool's quorum after startup, keeping RPC reads and
/// updates ordered. Validation must not write back thresholds from its concurrent RPC checks.
async fn refresh_quorum(shared: &Arc<Shared>) -> Result<(), Error> {
    let chain_key = shared.chain_key;
    let threshold = crate::retry::with_retries(&shared.cc3, &shared.token, |cc3| async move {
        cc3.quorum(chain_key).await
    })
    .await
    .map_err(Error::Rpc)?;

    tracing::info!(threshold, "🧮 quorum refreshed");
    shared.pool_send.note_quorum_change(threshold);
    // Keep the health p2p-isolation axis in step with the live quorum — it was armed once at
    // startup. Without this, a quorum dropping to 1 leaves a correctly-peerless sole attestor
    // flagged `Isolated` (restart loop: a fresh pod won't conjure peers), and a quorum growing
    // past 1 after a single-attestor start leaves isolation detection permanently disarmed.
    // `set_p2p_expected` is idempotent and re-stamps the peerless clock on the disarm→arm
    // transition, so re-arming gets a fresh grace window.
    shared.health.set_p2p_expected(threshold > 1);
    Ok(())
}

async fn handle_one(
    shared: &Arc<Shared>,
    stream_attestation: &mut stream::attestation::StreamAttestation,
    latest_cc3: &mut AttestationInfo,
    event: CcEvent,
) -> Result<(), Error> {
    match event {
        // 1] new finalized attestation on cc3
        CcEvent::BlockAttested(att) => {
            let info = AttestationInfo {
                height: att.header_number,
                digest: att.digest,
            };
            if info.height > latest_cc3.height {
                tracing::info!(height = info.height, digest = ?info.digest, "💾 cc3 finalized");
                *latest_cc3 = info;
                // 0. watchdog: the network is visibly finalizing attestations right now — an
                //    eligible attestor that produces nothing against this backdrop is stalled.
                shared.health.note_chain_attested();
                // 1. nudge eth stream
                stream_attestation.note_attestation_finalization(stream::util::AttestationInfo {
                    height: info.height,
                    digest: info.digest,
                });
                // 2. nudge pool
                shared
                    .pool_send
                    .note_attestation_finalization(info.height, info.digest);
                // 3. nudge proof cache
                shared.proof_cache.note_finalized(info.height);
                // 4. nudge watchers (validation task)
                let _ = shared.latest_finalized_tx.send(Some(info));
                // 5. metrics
                shared.metrics.set_attestation_finalized(info.height);
            }
        }

        // 2] new attestation interval
        CcEvent::AttestationIntervalChanged(_, raw) => {
            let Some(interval) = NonZero::new(raw) else {
                return Ok(());
            };
            tracing::info!(interval = %interval, "🔢 new attestation interval");
            *shared.interval_attestation.write() = interval;
            stream_attestation
                .note_attestation_interval_change(interval)
                .await;
            shared.pool_send.note_attestation_interval_change(interval);
        }

        // 3] new target sample size. Inert today: the quorum is `2/3+1` of `|ActiveAttestors|`
        // and does not read this value (it is reserved for RFC-0174 sortition). Refreshing the
        // quorum here is harmless and keeps the seam wired for when it does matter.
        CcEvent::TargetSampleSizeChanged(_, target) => {
            tracing::info!(target, "📏 new target sample size");
            refresh_quorum(shared).await?;
        }

        // 3b] new max catchup (block-count bound per continuity proof). Keep the off-chain view
        // in sync with runtime policy: the eth stream's root cache / proof length, the pool's
        // vote-admission window, and p2p pending-vote buffering all derive from this value.
        CcEvent::MaxCatchupChanged(_, raw) => {
            let Some(max_catchup) = NonZero::new(u64::from(raw)) else {
                return Ok(());
            };
            tracing::info!(max_catchup = %max_catchup, "🔢 new max catchup");
            *shared.max_catchup.write() = max_catchup;
            stream_attestation
                .note_max_catchup_change(max_catchup)
                .await;
            shared.pool_send.note_max_catchup_change(max_catchup);
        }

        // 4] attestor election
        CcEvent::AttestorsElected(_, attestors) => {
            tracing::info!("⏰ new attestor set");
            let eligible = attestors.contains(&shared.account_id);
            // Refresh our own committee view (BLS store + pool allow-set) BEFORE arming the
            // warm-up. `apply_eligibility`'s warm-up timer is what gates `can_attest` true, and
            // it must start from a current local view — otherwise a slow/retrying BLS fetch
            // could let the timer fire while we still verify against the previous committee,
            // dropping newly-elected peers' votes as UnknownAttestor (unrecoverable: gossipsub
            // marks them seen).
            shared
                .bls_store
                .note_attestors_elected(&shared.cc3, &shared.token, &attestors)
                .await
                .map_err(Error::Rpc)?;
            shared.pool_send.note_attestors_elected(attestors);
            // Membership is half of the quorum denominator — an election that grows or shrinks
            // the active set moves the threshold even though the target is unchanged.
            refresh_quorum(shared).await?;
            apply_eligibility(shared, eligible);
        }

        CcEvent::AttestorActivated(_, who) => {
            if who == shared.account_id {
                tracing::info!("🔋 activated");
            }
        }

        // Chill / kick removes the attestor from `ActiveAttestors` on-chain immediately. Refresh
        // BlsStore and the pool's allow-set on every chill/kick — not just for the local node —
        // so peers stop accepting gossip signed by the removed attestor's still-cached BLS key.
        // `AttestorsElected` is only emitted when the committee changes, so there is no
        // per-epoch event to fall back on; the `RandomnessChanged` reconcile below is the
        // periodic safety net if this refresh ever reads a lagging node.
        CcEvent::AttestorChilled(_, who) | CcEvent::AttestorKicked(who) => {
            let is_local = who == shared.account_id;
            if is_local {
                tracing::info!("🪫 local node deactivated/kicked");
            } else {
                tracing::info!(attestor = %who, "🪫 peer deactivated/kicked");
            }
            // Same path as the per-epoch reconcile below: refresh BLS store, pool allow-set and
            // the quorum (membership is half the denominator) from the authoritative on-chain set,
            // then arm eligibility. `staking::Kicked` carries no chain scope, so the event itself
            // is never trusted for `can_attest`.
            reconcile_committee_from_chain(shared).await?;

            // Nudge the p2p task to evict this attestor's peer from the routing table / drop the
            // connection. Sent *after* the `bls_store` refresh above so the p2p task's active-set
            // view (which drives its deny decision) already reflects the removal. Skip the local
            // node — there is no remote peer of our own to evict. A closed receiver (p2p task
            // gone / shutting down) is not fatal here, so the send error is ignored.
            if !is_local {
                let _ = shared
                    .peer_deactivated_tx
                    .send(attestor_primitives::AttestorId::from_public(who.0));
            }
        }

        CcEvent::AttestationChainGenesisBlockNumberSet(_, h) => {
            tracing::info!(genesis = h, "🎬 attestation chain genesis set");
        }

        CcEvent::CheckpointReached(_, cp) => {
            tracing::info!(height = cp.block_number, digest = ?cp.digest, "🛟 checkpoint");
        }

        CcEvent::CheckpointIntervalChanged(_, i) => {
            tracing::info!(interval = i, "🔢 new checkpoint interval");
        }

        // Every epoch boundary, re-derive the committee view from storage. `AttestorsElected`
        // fires only on membership changes, so this once-per-epoch reconcile is what corrects a
        // view that went stale through a missed event or a chill/kick refresh that read a lagging
        // node. Cheap: one storage read plus the BLS key fetches for the set.
        CcEvent::RandomnessChanged((epoch, _)) => {
            tracing::info!(epoch, "🎲 new epoch");
            reconcile_committee_from_chain(shared).await?;
        }

        CcEvent::RevertedAttestationChainTo(_, height, digest) => {
            tracing::warn!(height, ?digest, "💥 attestation chain reversion");
            let info = AttestationInfo { height, digest };
            *latest_cc3 = info;
            shared
                .pool_send
                .note_attestation_chain_reversion(height, digest);
            shared.proof_cache.clear();
            stream_attestation
                .note_attestation_chain_reversion(stream::util::AttestationInfo { height, digest })
                .await;
            let _ = shared.latest_finalized_tx.send(Some(info));
            // Roll local-production tracking back to the revert height too. Otherwise p2p keeps
            // the pre-revert (higher) height as "locally produced", skewing its pending-vote
            // pruning until production re-produces past the rollback.
            let _ = shared.local_produced_tx.send(Some(height));

            // Reset metrics so operator dashboards reflect the post-rollback state immediately
            // rather than continuing to show stale local/finalized heights and a misleading lag.
            // Without this, the gauges would only drift back to reality on the next emit_local
            // / cc3 finalization, which can be several blocks away after a deep revert.
            shared.metrics.set_attestation_finalized(height);
            shared.metrics.set_attestation_local(height);
            shared.metrics.update_attestation_lag_eth(
                height,
                height,
                shared.attestation_interval(),
            );
            shared.metrics.update_attestation_lag_cc3(
                height,
                height,
                shared.attestation_interval(),
            );
            // Re-arm the health stall axis: the rebuild from the revert height to the first
            // post-revert emit_local can exceed the stall deadline while peers keep finalizing
            // (uneven recovery), which is exactly axis 2's trigger shape — without this the pod
            // gets restarted mid-catch-up and re-does the same rebuild.
            shared.health.rearm_stall_window();
        }
    }

    Ok(())
}

// ------------------------------ [ Helper: wait for genesis maturity ] ------------------------- //

/// Block until `genesis` is at or below the source chain's mature height. `false` means shutdown
/// was requested while waiting.
async fn wait_for_genesis_maturity(shared: &Shared, genesis: attestor_primitives::Height) -> bool {
    wait_until_mature(
        &shared.token,
        shared.eth_head_poll_interval,
        genesis,
        || async {
            let head = shared.eth.get_last_block().await?;
            shared.maturity.mature_height(&shared.eth, head).await
        },
    )
    .await
}

/// Poll `mature_height` every `poll` until it reaches `target`. A genesis above the current tip
/// is simply not mature yet, and an unresolvable mature height is retried: neither is an init
/// failure, since a restart loop would only redo the same wait.
async fn wait_until_mature<F, Fut>(
    token: &tokio_util::sync::CancellationToken,
    poll: std::time::Duration,
    target: attestor_primitives::Height,
    mut mature_height: F,
) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Option<attestor_primitives::Height>, eth::Error>>,
{
    loop {
        let resolved = tokio::select! {
            _ = token.cancelled() => return false,
            resolved = mature_height() => resolved,
        };
        match resolved {
            Ok(Some(mature)) if mature >= target => return true,
            Ok(mature) => tracing::info!(
                target,
                ?mature,
                "⏳ genesis block is not mature yet — waiting before signing it"
            ),
            Err(err) => tracing::warn!(
                target,
                %err,
                "⏳ could not resolve the mature height — retrying"
            ),
        }
        tokio::select! {
            _ = token.cancelled() => return false,
            () = tokio::time::sleep(poll) => {}
        }
    }
}

// ------------------------------ [ Helper: wait for the genesis step ] ------------------------- //
//
// Drains CC3 events on the production task's own subscription until the genesis bootstrap can
// move on. Used by the genesis path to learn the runtime-committed digest before initializing the
// eth attestation stream's prev-digest, and (with `await_maturity`) to hold the genesis vote back
// until the block is mature and this attestor eligible, while still applying committee / config
// events, noticing a genesis that peers finalized, and failing if the genesis height is re-set.

/// Why [`wait_for_genesis`] returned.
enum GenesisWait {
    /// The first `BlockAttested` at/above genesis: the real `(height, digest)`.
    Attested(AttestationInfo),
    /// The genesis block is mature and this attestor may attest (only with `await_maturity`).
    Mature,
    /// Shutdown was requested while waiting (an intentional cancellation, not a failure).
    Shutdown,
}

/// What one cc3 event means to the genesis wait.
enum GenesisStep {
    /// A `BlockAttested` at/above genesis: the real `(height, digest)`.
    Adopt(AttestationInfo),
    /// A `BlockAttested` below genesis: nothing to apply.
    Ignore,
    /// The chain's genesis height was re-set, so this wait is for a stale target.
    GenesisChanged(attestor_primitives::Height),
    /// Everything else (elections, chill/kick, config changes) goes through `handle_one`.
    Apply(CcEvent),
}

fn genesis_step(
    event: CcEvent,
    target: attestor_primitives::Height,
    chain_key: attestor_primitives::ChainKey,
) -> GenesisStep {
    match event {
        CcEvent::BlockAttested(att) if att.header_number >= target => {
            GenesisStep::Adopt(AttestationInfo {
                height: att.header_number,
                digest: att.digest,
            })
        }
        CcEvent::BlockAttested(_) => GenesisStep::Ignore,
        CcEvent::AttestationChainGenesisBlockNumberSet(key, new)
            if key == chain_key && new != target =>
        {
            GenesisStep::GenesisChanged(new)
        }
        other => GenesisStep::Apply(other),
    }
}

/// Resolve once `can_attest` is true; `false` only if its sender is gone. A chilled, kicked or
/// still-warming-up attestor must not sign: peers drop votes they cannot yet verify, and
/// gossipsub never redelivers them.
async fn wait_until_eligible(can_attest: &mut tokio::sync::watch::Receiver<bool>) -> bool {
    if !*can_attest.borrow() {
        tracing::info!(
            "⏳ genesis block is mature but this attestor is not eligible to attest yet"
        );
    }
    can_attest.wait_for(|eligible| *eligible).await.is_ok()
}

async fn wait_for_genesis(
    shared: &Arc<Shared>,
    events: &mut stream::cc3::StreamCC3,
    stream_attestation: &mut stream::attestation::StreamAttestation,
    target: attestor_primitives::Height,
    await_maturity: bool,
) -> Result<GenesisWait, Error> {
    use futures::{StreamExt as _, TryStreamExt as _};

    // Without `await_maturity` the gate never opens. Polled alongside the events below, so a long
    // wait neither starves the cc3 stream nor is restarted by each batch. Eligibility is read
    // after maturity: chill/kick/election events applied during the wait may have changed it.
    let gate = async {
        if await_maturity {
            wait_for_genesis_maturity(shared, target).await
                && wait_until_eligible(&mut shared.can_attest_rx.clone()).await
        } else {
            std::future::pending().await
        }
    };
    tokio::pin!(gate);

    // Scratch cursor for `handle_one`'s `BlockAttested` / reversion arms; the caller keeps the
    // authoritative `latest_cc3` from this function's return value.
    let mut scratch = AttestationInfo::default();

    const TICK_SECS: u64 = 5;
    // With the p2p outbox re-gossiping unfinalized votes every 30s, a healthy quorum recovers
    // from any lost genesis publish within about a minute. A wait this long therefore means the
    // bootstrap is genuinely stuck (peers down, quorum unreachable, vote mismatch) — escalate to
    // ERROR once per minute so log gates and alerting surface it, instead of the deadlock hiding
    // behind an INFO ticker until some downstream test times out.
    const ESCALATE_AFTER_SECS: u64 = 5 * 60;

    let mut tick = tokio::time::interval(std::time::Duration::from_secs(TICK_SECS));
    let mut waited_secs: u64 = 0;
    loop {
        tokio::select! {
            // Biased, events before the gate: a genesis that peers already finalized must win
            // over signing a stale vote.
            biased;
            _ = shared.token.cancelled() => return Ok(GenesisWait::Shutdown),
            maybe_batch = events.next() => {
                // Stream end = permanent backfill error (see the main-loop arm) — fail fast.
                let Some(mut batch) = maybe_batch else {
                    return Err(Error::Cc3Stream(stream::cc3::Error::EndOfStream));
                };
                // Keep the watchdog fed during a slow genesis wait (can exceed the startup grace
                // on a slow chain) — batches are still arriving, so we're alive, not wedged.
                shared.health.note_progress();
                let mut adopted = None;
                while let Some(event) = batch.try_next().await? {
                    match genesis_step(event, target, shared.chain_key) {
                        // The first BlockAttested at/above genesis carries the runtime-committed
                        // digest the caller needs. The rest of the block is still applied below,
                        // then the caller runs the finalization bookkeeping (it does more than
                        // `handle_one`'s BlockAttested arm — pool floor, proof cache, watchdog).
                        GenesisStep::Adopt(info) => {
                            if adopted.is_none() {
                                tracing::info!(
                                    height = info.height,
                                    digest = ?info.digest,
                                    "✅ genesis attestation finalized on-chain"
                                );
                                adopted = Some(info);
                            }
                        }
                        GenesisStep::Ignore => {}
                        // A re-set genesis makes this wait stale and `shared.genesis` is fixed at
                        // startup: fail so the supervisor restarts the process, which re-reads it.
                        GenesisStep::GenesisChanged(new) => {
                            return Err(Error::Init(anyhow::anyhow!(
                                "attestation genesis changed from {target} to {new} while bootstrapping"
                            )));
                        }
                        // Committee, chill/kick and config changes must still be applied while we
                        // wait — dropping them left `bls_store`, pool membership and threshold
                        // config stale. Same handler as the steady-state loop.
                        GenesisStep::Apply(other) => {
                            handle_one(shared, stream_attestation, &mut scratch, other).await?;
                        }
                    }
                }
                if let Some(info) = adopted {
                    return Ok(GenesisWait::Attested(info));
                }
            }
            matured = &mut gate => {
                return Ok(if matured { GenesisWait::Mature } else { GenesisWait::Shutdown });
            }
            _ = tick.tick() => {
                waited_secs += TICK_SECS;
                // The maturity wait logs its own progress and can take minutes; the stuck-
                // bootstrap escalation only applies once our genesis vote is out.
                if !await_maturity {
                    if waited_secs >= ESCALATE_AFTER_SECS && waited_secs % 60 == 0 {
                        tracing::error!(
                            target,
                            waited_secs,
                            "⏲️ genesis BlockAttested still missing after prolonged waiting — the attestation chain is not bootstrapping. Check that a quorum of attestors for this chain key is running and meshed (each re-gossips its genesis vote every 30s, so a healthy quorum recovers in under a minute)"
                        );
                    } else {
                        tracing::info!(target, waited_secs, "⏲️ still waiting for genesis BlockAttested...");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        eligibility_action, genesis_step, wait_until_eligible, wait_until_mature,
        EligibilityAction, GenesisStep,
    };
    use attestor_primitives::Digest;
    use cc_client::attestation::{BlockAttestedMetadata, CcEvent};
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    type Step = Result<Option<u64>, eth::Error>;

    const CHAIN: u64 = 7;

    fn attested(height: u64) -> CcEvent {
        CcEvent::BlockAttested(BlockAttestedMetadata {
            chain_key: CHAIN,
            header_number: height,
            digest: Digest::repeat_byte(9),
        })
    }

    #[test]
    fn genesis_step_adopts_attestations_at_or_above_genesis() {
        for height in [100, 101, 5_000] {
            match genesis_step(attested(height), 100, CHAIN) {
                GenesisStep::Adopt(info) => {
                    assert_eq!((info.height, info.digest), (height, Digest::repeat_byte(9)));
                }
                _ => panic!("an attestation at {height} must end the wait"),
            }
        }
    }

    #[test]
    fn genesis_step_ignores_attestations_below_genesis() {
        assert!(matches!(
            genesis_step(attested(99), 100, CHAIN),
            GenesisStep::Ignore
        ));
    }

    #[test]
    fn genesis_step_flags_a_reset_genesis_height() {
        let set = |key, height| CcEvent::AttestationChainGenesisBlockNumberSet(key, height);
        assert!(matches!(
            genesis_step(set(CHAIN, 250), 100, CHAIN),
            GenesisStep::GenesisChanged(250)
        ));
        // Re-setting the same height, or another chain's genesis, changes nothing for this wait.
        assert!(matches!(
            genesis_step(set(CHAIN, 100), 100, CHAIN),
            GenesisStep::Apply(_)
        ));
        assert!(matches!(
            genesis_step(set(CHAIN + 1, 250), 100, CHAIN),
            GenesisStep::Apply(_)
        ));
    }

    #[test]
    fn genesis_step_applies_everything_else() {
        let event = CcEvent::AttestationIntervalChanged(CHAIN, 10);
        assert!(matches!(
            genesis_step(event, 100, CHAIN),
            GenesisStep::Apply(CcEvent::AttestationIntervalChanged(CHAIN, 10))
        ));
    }

    #[tokio::test]
    async fn eligible_attestor_is_released_at_once() {
        let (_tx, mut rx) = tokio::sync::watch::channel(true);
        assert!(wait_until_eligible(&mut rx).await);
    }

    /// A chilled, kicked or warming-up attestor holds its genesis vote until eligible again.
    #[tokio::test]
    async fn ineligible_attestor_holds_until_eligible() {
        let (tx, mut rx) = tokio::sync::watch::channel(false);
        let held = tokio::time::timeout(Duration::from_millis(30), wait_until_eligible(&mut rx));
        assert!(held.await.is_err());
        tx.send(true).unwrap();
        assert!(wait_until_eligible(&mut rx).await);
    }

    #[tokio::test]
    async fn eligibility_wait_ends_when_the_sender_is_gone() {
        let (tx, mut rx) = tokio::sync::watch::channel(false);
        drop(tx);
        assert!(!wait_until_eligible(&mut rx).await);
    }

    /// Drive `wait_until_mature` with a scripted sequence of mature-height answers. Returns the
    /// outcome and how many answers were consumed.
    async fn run_script(
        token: &CancellationToken,
        target: u64,
        script: Vec<Step>,
    ) -> (bool, usize) {
        let total = script.len();
        let script = Mutex::new(VecDeque::from(script));
        let matured = wait_until_mature(token, Duration::from_millis(1), target, || {
            let next = script.lock().unwrap().pop_front();
            async move { next.expect("script exhausted before maturity") }
        })
        .await;
        let consumed = total - script.lock().unwrap().len();
        (matured, consumed)
    }

    #[tokio::test]
    async fn mature_genesis_is_released_without_waiting() {
        let token = CancellationToken::new();
        assert_eq!(
            run_script(&token, 100, vec![Ok(Some(100))]).await,
            (true, 1)
        );
    }

    /// A genesis at/above the mature height is waited on, not signed or treated as an init error.
    #[tokio::test]
    async fn unmatured_genesis_is_waited_on_until_it_matures() {
        let token = CancellationToken::new();
        let script = vec![Ok(None), Ok(Some(98)), Ok(Some(99)), Ok(Some(100))];
        assert_eq!(run_script(&token, 100, script).await, (true, 4));
    }

    #[tokio::test]
    async fn unresolvable_mature_height_is_retried() {
        let token = CancellationToken::new();
        let script = vec![Err(eth::Error::FailedToGetBlock(1)), Ok(Some(100))];
        assert_eq!(run_script(&token, 100, script).await, (true, 2));
    }

    #[tokio::test]
    async fn shutdown_ends_the_wait() {
        let token = CancellationToken::new();
        let canceller = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            canceller.cancel();
        });
        let matured = wait_until_mature(&token, Duration::from_secs(3600), 100, || async {
            Ok(Some(1))
        })
        .await;
        assert!(!matured);
    }

    /// The full eligibility transition matrix. Regression pin for the warm-up logic: the bug that
    /// motivated the target guard was unrelated `AttestorKicked` events (target unchanged) being
    /// treated as transitions and cancelling a pending warm-up.
    #[test]
    fn eligibility_action_matrix() {
        // Target unchanged → NoOp (no generation bump, so a pending warm-up survives).
        assert_eq!(
            eligibility_action(true, true),
            EligibilityAction::NoOp,
            "still-eligible (e.g. an unrelated kick) must not disturb a warm-up"
        );
        assert_eq!(
            eligibility_action(false, false),
            EligibilityAction::NoOp,
            "still-ineligible must be inert"
        );
        // Genuine transitions.
        assert_eq!(
            eligibility_action(false, true),
            EligibilityAction::WarmUpEnable,
            "re-election must warm up before enabling"
        );
        assert_eq!(
            eligibility_action(true, false),
            EligibilityAction::DisableNow,
            "chill/kick must disable immediately"
        );
    }
}
