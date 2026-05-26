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

    let max_parallelism = std::thread::available_parallelism()
        .ok()
        .and_then(|n| n.get().checked_sub(common::constants::WORKER_COUNT + 1))
        .and_then(NonZero::new)
        .unwrap_or(NonZero::<usize>::MIN);

    let roots_cfg = stream::eth::roots::ConfigBuilder::new()
        .with_client(shared.eth.clone())
        .with_start_height(start_height)
        .with_finalization_lag(shared.maturity_delay)
        .with_max_concurrency(common::constants::MAX_CONCURRENT_RPC_CALLS)
        .with_max_parallelism(max_parallelism)
        .build();
    let stream_roots = stream::eth::StreamRoots::new(roots_cfg).await;

    let tip_cfg = stream::eth::tip::ConfigBuilder::new()
        .with_client(shared.eth.clone())
        .with_finalization_lag(shared.maturity_delay)
        .with_start_height(start_height)
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
        .with_max_catchup(common::constants::MAX_CATCHUP)
        .build();
    let mut stream_attestation = stream::attestation::StreamAttestation::new(attestation_cfg);

    // ----------------------------------* cc3 stream *---------------------------------------- //
    // Opened before the (optional) genesis step so we can wait for the genesis BlockAttested
    // event on the same subscription we use in the main loop.

    let cc3_cfg = stream::cc3::ConfigBuilder::new()
        .with_cc3((*shared.cc3).clone())
        .with_chain_key(shared.chain_key)
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
        tracing::info!(genesis, "👶 generating genesis attestation");
        let block = shared
            .eth
            .get_block(genesis, usc_abi_encoding::common::EncodingVersion::V1)
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
        let info = wait_for_block_attested(&shared, &mut events, genesis).await?;
        stream_attestation.note_attestation_finalization(stream::util::AttestationInfo {
            height: info.height,
            digest: info.digest,
        });
        let _ = shared.latest_finalized_tx.send(Some(info));
        shared.proof_cache.note_finalized(info.height);
        shared.metrics.set_attestation_finalized(info.height);
        info
    };

    // ----------------------------------* main loop *----------------------------------------- //

    loop {
        let can_attest = *shared.can_attest_rx.borrow();
        tokio::select! {
            biased;
            _ = shared.token.cancelled() => return Ok(()),

            Some(batch) = events.next() => {
                handle_cc3_batch(&shared, &mut stream_attestation, &mut latest_cc3, batch).await?;
            }

            Some(attestation) = stream_attestation.next(), if can_attest => {
                emit_local(&shared, &attestation).await;
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

    // Send to gossip. The gossip task will broadcast if/when the mesh is ready.
    if let Err(err) = shared.gossip_tx.send(vote).await {
        tracing::warn!(?err, "📭 gossip channel closed");
    }

    // metrics
    shared
        .metrics
        .update_attestation_delay_production(now.elapsed());
    shared.metrics.set_attestation_local(height);
}

// ----------------------------------- [ Handle CC3 events ] ------------------------------------ //

async fn handle_cc3_batch(
    shared: &Arc<Shared>,
    stream_attestation: &mut stream::attestation::StreamAttestation,
    latest_cc3: &mut AttestationInfo,
    mut batch: stream::cc3::StreamEvents,
) -> Result<(), Error> {
    while let Some(event) = batch.try_next().await? {
        handle_one(shared, stream_attestation, latest_cc3, event).await?;
    }
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

        // 3] new sample size
        CcEvent::TargetSampleSizeChanged(_, target) => {
            tracing::info!(target, "📏 new target sample size");
            shared.pool_send.note_target_sample_size_change(target);
        }

        // 4] attestor election
        CcEvent::AttestorsElected(_, attestors) => {
            tracing::info!("⏰ new attestor set");
            let eligible = attestors.contains(&shared.account_id);
            // can_attest toggle wakes any watcher (production select, validation submit gate).
            let _ = shared.can_attest_tx.send(eligible);
            shared
                .bls_store
                .note_attestors_elected(&shared.cc3, &shared.token, &attestors)
                .await
                .map_err(Error::Rpc)?;
            shared.pool_send.note_attestors_elected(attestors);
        }

        CcEvent::AttestorActivated(_, who) => {
            if who == shared.account_id {
                tracing::info!("🔋 activated");
            }
        }

        CcEvent::AttestorChilled(_, who) | CcEvent::AttestorKicked(who)
            if who == shared.account_id =>
        {
            tracing::info!("🪫 deactivated/kicked");
            let _ = shared.can_attest_tx.send(false);
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

        CcEvent::RandomnessChanged((epoch, _)) => {
            tracing::info!(epoch, "🎲 new epoch");
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
        }

        _ => {}
    }

    Ok(())
}

// --------------------------- [ Helper: wait for a BlockAttested event ] ----------------------- //
//
// Drains CC3 events on the production task's own subscription until BlockAttested(height ≥ N)
// arrives. Returns the real `(height, digest)`. Used by the genesis bootstrap path to learn the
// runtime-committed digest before initializing the eth attestation stream's prev-digest.

async fn wait_for_block_attested(
    shared: &Arc<Shared>,
    events: &mut stream::cc3::StreamCC3,
    target: attestor_primitives::Height,
) -> Result<AttestationInfo, Error> {
    use cc_client::attestation::CcEvent;
    use futures::{StreamExt as _, TryStreamExt as _};

    let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        tokio::select! {
            _ = shared.token.cancelled() => {
                return Err(Error::Init(anyhow::anyhow!("cancelled while waiting for genesis")));
            }
            _ = tick.tick() => {
                tracing::info!(target, "⏲️ still waiting for genesis BlockAttested...");
            }
            Some(mut batch) = events.next() => {
                while let Some(event) = batch.try_next().await? {
                    if let CcEvent::BlockAttested(att) = event {
                        if att.header_number >= target {
                            tracing::info!(
                                height = att.header_number,
                                digest = ?att.digest,
                                "✅ genesis attestation finalized on-chain"
                            );
                            return Ok(AttestationInfo {
                                height: att.header_number,
                                digest: att.digest,
                            });
                        }
                    }
                }
            }
        }
    }
}
