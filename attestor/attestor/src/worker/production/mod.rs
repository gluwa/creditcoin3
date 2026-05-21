//! A [`Worker`] thread responsible for the production of new attestations.
//!
//! # Ethereum Data
//!
//! The production worker keeps track of source chain finality via the [attestation stream], which
//! abstracts away a of lot the complexity associated with the generation of new attestations. When
//! a new source chain block is noticed and if it is past the `maturity_delay`, this triggers the
//! production of a new [`Attestation`].
//!
//! Attestations produced this way are then sent to the [p2p worker] for dissemination and the
//! [validation worker] for validation and submission once quorum has been reached.
//!
//! ## Attestation production flow
//!
#![doc = include_str!("../../../../mermaid.html")]
//! <pre class="mermaid">
//! sequenceDiagram
//!     box Networks
//!         participant Eth
//!     end
//!     box Thread 2
//!         participant Eth Chain Listener
//!         participant CC3 Chain Listener
//!         participant Rebroadcast
//!         participant Production Worker
//!     end
//!     box Shared
//!         participant Attestation Pool
//!     end
//!     box Thread 3
//!         participant P2P Worker
//!     end
//!     box Thread 4
//!         participant Validation Worker
//!     end
//!     box Thread 5..n
//!         participant Rayon Thread Pool
//!     end
//!
//!     loop Production
//!         Production Worker ->> Eth Chain Listener: Polls
//!
//!         activate Eth Chain Listener
//!         Eth Chain Listener -->> Eth: Polls
//!         deactivate Eth Chain Listener
//!
//!         activate Eth
//!         Eth -->> Eth Chain Listener: New block
//!         deactivate Eth
//!
//!         activate Eth Chain Listener
//!         Eth Chain Listener ->> Production Worker: Notify
//!         deactivate Eth Chain Listener
//!
//!         activate Production Worker
//!         Production Worker ->> CC3 Chain Listener: Generate Attestation
//!         deactivate Production Worker
//!
//!         activate CC3 Chain Listener
//!         CC3 Chain Listener ->> Rayon Thread Pool: Compute Continuity Proof
//!         activate Rayon Thread Pool
//!         Rayon Thread Pool ->> CC3 Chain Listener: Continuity Proof
//!         deactivate Rayon Thread Pool
//!         CC3 Chain Listener ->> Production Worker: Attestation
//!         deactivate CC3 Chain Listener
//!
//!         activate Production Worker
//!         Production Worker ->> Attestation Pool: Store attestation
//!         Production Worker ->> P2P Worker: Send attestation
//!         Production Worker ->> Validation Worker: Send attestation
//!         deactivate Production Worker
//!     end
//! </pre>
//!
//! # CC3 Data
//!
//! The production worker also listens to changes in execution chain state by subscribing to cc3
//! events. These events are then forwarded for further handling.
//!
//! [`Worker`]: crate::worker::Worker
//! [attestation stream]: crate::stream::attestation
//! [attestation pool]: attestation_pool
//! [`Attestation`]: crate::common::types::Attestation
//! [p2p worker]: crate::worker::p2p
//! [validation worker]: crate::worker::validation
//! [`Quorum`]: attestation_pool::Quorum

mod error;

pub use error::*;
use user::prelude::*;

// -------------------------------------- [ Configuration ] ------------------------------------ //

/// Attestation production configuration. This includes options to initialize cross-tread
/// communication channels, set up [chain listeners] and store identifying information about an
/// attestor, such as its account id.
#[derive(builder::Builder)]
pub struct Config {
    stream_attestation: stream::attestation::StreamAttestation,
    stream_cc3: stream::cc3::StreamCC3,
    cc3: cc_client::Client,
    bls: std::sync::Arc<crate::bls::BlsStore>,

    sender_p2p: tokio::sync::broadcast::Sender<common::types::Attestation>,
    sender_validation: attestation_pool::AttestationPoolSender,

    interval_attestation: std::num::NonZero<attestor_primitives::Height>,
    attestation_latest_cc3: std::sync::Arc<std::sync::atomic::AtomicU64>,
    can_attest: std::sync::Arc<std::sync::atomic::AtomicBool>,

    start_height: attestor_primitives::Height,
    account_id: cc_client::AccountId32,
    metrics: metrics::Metrics,
}

// ----------------------------------------- [ Worker ] ---------------------------------------- //

pub(crate) struct WorkerAttestationProduction {
    // CHAIN LISTENERS
    stream_attestation: stream::attestation::StreamAttestation,
    stream_cc3: stream::cc3::StreamCC3,
    cc3: cc_client::Client,
    bls: std::sync::Arc<crate::bls::BlsStore>,

    // MESSAGE CHANNELS
    sender_p2p: tokio::sync::broadcast::Sender<common::types::Attestation>,
    sender_validation: attestation_pool::AttestationPoolSender,

    // ATTESTATION DATA
    attestation_local: attestor_primitives::Height,
    attestation_latest_cc3: std::sync::Arc<std::sync::atomic::AtomicU64>,
    attestation_interval: std::num::NonZero<attestor_primitives::Height>,

    // METRICS
    metrics: metrics::Metrics,

    // ATTESTOR DATA
    account_id: cc_client::AccountId32,
    can_attest: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl WorkerAttestationProduction {
    pub(crate) fn new(config: Config) -> Self {
        Self {
            stream_attestation: config.stream_attestation,
            stream_cc3: config.stream_cc3,
            cc3: config.cc3,
            bls: config.bls,

            sender_p2p: config.sender_p2p,
            sender_validation: config.sender_validation,

            attestation_local: config.start_height,
            attestation_latest_cc3: config.attestation_latest_cc3,
            attestation_interval: config.interval_attestation,

            metrics: config.metrics,

            account_id: config.account_id,
            can_attest: config.can_attest,
        }
    }
}

// ---------------------------------------- [ Main loop ] -------------------------------------- //

impl super::Worker for WorkerAttestationProduction {
    type Error = Error;

    #[tracing::instrument(name = "production", skip_all)]
    async fn task(
        mut self,
        mut shutdown: std::pin::Pin<Box<impl std::future::Future<Output = ()>>>,
    ) -> crate::worker::Exit<Error> {
        use futures::StreamExt as _;

        loop {
            let can_attest = self.can_attest.load(std::sync::atomic::Ordering::Acquire);

            let runtime_version = self.cc3.api().runtime_version();
            let spec_version = runtime_version.spec_version;
            let transaction_version = runtime_version.transaction_version;

            // Added for ease of debugging and visibility into the production loop, especially around runtime upgrades.
            tracing::info!(spec_version = %spec_version, transaction_version = %transaction_version, can_attest, "Handling next production event");

            tokio::select! {
                biased;

                _ = &mut shutdown => {
                    break Err(Interrupt::Stop);
                }
                Some(events) = self.stream_cc3.next() => {
                    self.handle_event_cc3(events).await?;
                }
                Some(attestation) = self.stream_attestation.next(), if can_attest => {
                    self.handle_event_attestation(attestation).await?;
                }
            }
        }
    }
}

// ----------------------------------------- [ Events ] ---------------------------------------- //

impl WorkerAttestationProduction {
    // ----------------------------------------* Eth events *--------------------------------------

    async fn handle_event_attestation(
        &mut self,
        attestation: stream::attestation::Attestation,
    ) -> Result<(), Interrupt<Error>> {
        let now = std::time::Instant::now();

        let height = attestation.header_number();
        let digest = attestation.digest();
        // No previous digest means we will log `0x000...000` as the previous digest
        let digest_prev = attestation
            .prev_digest()
            .unwrap_or_else(sp_core::H256::zero);
        let attestor_id = attestation.attestor.account_id();

        tracing::info!(
            ?digest,
            ?digest_prev,
            height,
            %attestor_id,
            elapsed_ms = now.elapsed().as_millis(),
            "📡 Generated attestation"
        );

        self.metrics
            .update_attestation_delay_production(now.elapsed());

        self.metrics.set_attestation_local(height);

        self.metrics.update_attestation_lag_eth(
            attestation.header_number(),
            self.stream_attestation.latest_tip(),
            self.attestation_interval,
        );
        self.metrics.update_attestation_lag_cc3(
            attestation.header_number(),
            self.attestation_latest_cc3
                .load(std::sync::atomic::Ordering::Acquire),
            self.attestation_interval,
        );

        tracing::info!(
            ?digest,
            height,
            %attestor_id,
            "🗳️ Sending local attestation over for validation"
        );

        // From the tokio docs:
        //
        // > A send operation can only fail if there are no active receivers, implying that the
        // > message could never be received.
        //
        // This can happen during shutdown and does not represent a failing case!
        _ = self.sender_p2p.send(attestation.clone());

        if let Err(err) = self.sender_validation.send(attestation).transpose() {
            err.log_error(digest);
        }

        self.attestation_local = height;

        Ok(())
    }

    // ----------------------------------------* CC3 events *--------------------------------------

    async fn handle_event_cc3(
        &mut self,
        mut events: stream::cc3::StreamEvents,
    ) -> Result<(), Interrupt<Error>> {
        use futures::TryStreamExt as _;

        while let Some(event) = events.try_next().await.map_interrupt(Error::CC3)? {
            match event {
                // CASE 1] NEW ATTESTATION
                cc_client::attestation::CcEvent::BlockAttested(attestation) => {
                    let digest = attestation.digest;
                    let height = attestation.header_number;
                    let attestation_latest_cc3 = stream::util::AttestationInfo { digest, height };

                    tracing::info!(height, ?digest, "💾 New execution chain attestation");

                    if attestation_latest_cc3.height
                        > self
                            .attestation_latest_cc3
                            .load(std::sync::atomic::Ordering::Acquire)
                    {
                        self.attestation_latest_cc3
                            .store(height, std::sync::atomic::Ordering::Release);

                        // 1. Chain Listener - Eth
                        //
                        // This is ensure that we keep producing new attestation starting from the
                        // latest finalized on-chain attestation.
                        self.stream_attestation
                            .note_attestation_finalization(attestation_latest_cc3);

                        // 2. Update the attestation pool
                        //
                        // As an edge case, it is possible that we have already generated past
                        // attestations which have not yet been consumed by the validation thread. This
                        // can happen if the production thread is generating attestations faster than
                        // the validation thread can check new quorums. We remove those attestations
                        // here and also update the target block height (if necessary, it is also
                        // possible that we are in advance of the execution chain in which case we do
                        // not want to update the target height and this a no-op).
                        if let Err(err) = self
                            .sender_validation
                            .note_attestation_finalization(attestation_latest_cc3)
                        {
                            err.log_error(attestation_latest_cc3.digest);
                        };

                        // 5. Metrics
                        //
                        // Update attestation production metrics
                        self.metrics.set_attestation_finalized(height);
                        self.metrics.update_attestation_lag_cc3(
                            self.attestation_local,
                            height,
                            self.attestation_interval,
                        );
                    }
                }

                // CASE 2] NEW TARGET SAMPLE SIZE
                cc_client::attestation::CcEvent::TargetSampleSizeChanged(
                    _chain_key,
                    target_sample_size,
                ) => {
                    tracing::info!(target_sample_size, "📏 New target sample size");

                    self.sender_validation
                        .note_target_sample_size_change(target_sample_size);
                }

                // CASE 3] NEW ATTESTATION INTERVAL
                cc_client::attestation::CcEvent::AttestationIntervalChanged(
                    _chain_key,
                    interval,
                ) => {
                    tracing::info!(interval, "🔢 New source chain attestation interval");

                    let Some(interval) =
                        std::num::NonZero::<attestor_primitives::Height>::new(interval)
                    else {
                        return Ok(());
                    };

                    let attestation_latest_cc3 = self
                        .attestation_latest_cc3
                        .load(std::sync::atomic::Ordering::Acquire);

                    // 1. Chain listener - Eth
                    //
                    // Catchup to the new target height and update the attestation interval.
                    self.stream_attestation
                        .note_attestation_interval_change(interval)
                        .await;

                    // 2. Attestation pool
                    //
                    // Update quorum validation to expect the new target height and attestation
                    // interval.
                    self.sender_validation
                        .note_attestation_interval_change(interval);

                    // 3. Production
                    //
                    // Update local state
                    self.attestation_interval = interval;

                    // 4. Metrics
                    self.metrics.update_attestation_lag_eth(
                        attestation_latest_cc3,
                        self.stream_attestation.latest_tip(),
                        interval,
                    );
                    self.metrics.update_attestation_lag_cc3(
                        attestation_latest_cc3,
                        attestation_latest_cc3,
                        interval,
                    );
                }

                cc_client::attestation::CcEvent::CheckpointIntervalChanged(
                    _chain_key,
                    interval,
                ) => {
                    tracing::info!(interval, "🔢 New source chain checkpoint interval");
                }

                // CASE 4] NEW ATTESTATION CHECKPOINT
                cc_client::attestation::CcEvent::CheckpointReached(_chain_key, checkpoint) => {
                    tracing::info!(
                        height = checkpoint.block_number,
                        digest = ?checkpoint.digest,
                        "🛟 New execution chain attestation checkpoint"
                    )
                }

                // CASE 5] NEW EPOCH
                cc_client::attestation::CcEvent::RandomnessChanged((epoch, _randomness)) => {
                    tracing::info!(epoch, "🎲 New epoch rotation");
                }

                // CASE 6] ATTESTOR ELECTION
                cc_client::attestation::CcEvent::AttestorsElected(_chain_key, attestors) => {
                    tracing::info!("⏰ New attestors elected");

                    // 1. Attestor status
                    //
                    // Update local attestation eligibility.
                    if attestors.contains(&self.account_id) {
                        self.can_attest
                            .store(true, std::sync::atomic::Ordering::Release);
                        tracing::info!(
                            attestor_id = %self.account_id,
                            "☀️ Attestor is eligible for production"
                        );
                    } else {
                        self.can_attest
                            .store(false, std::sync::atomic::Ordering::Release);
                        tracing::info!(
                            attestor_id = %self.account_id,
                            "🛏️ Waiting for attestor to be elected"
                        );
                    }

                    // 2. Update bls keys
                    //
                    // Update the set of BLS keys for use by the p2p worker
                    self.bls
                        .note_attestors_elected(&mut self.cc3, &attestors)
                        .await
                        .map_interrupt(Error::Bls)?;

                    // 3. Attestor validation
                    //
                    // Update the set of legal attestors in the attestation pool.
                    self.sender_validation.note_attestors_elected(attestors);
                }

                // CASE 7] ATTESTOR ACTIVATION
                cc_client::attestation::CcEvent::AttestorActivated(_chain_key, attestor) => {
                    if attestor == self.account_id {
                        tracing::info!(
                            attestor_id = %self.account_id,
                            "🔋 Attestor has been activated"
                        );
                    }
                }

                // CASE 8] ATTESTOR DEACTIVATION
                cc_client::attestation::CcEvent::AttestorChilled(_chain_key, attestor) => {
                    if attestor == self.account_id {
                        self.can_attest
                            .store(false, std::sync::atomic::Ordering::Release);
                        tracing::info!(
                            attestor_id = %self.account_id,
                            "🪫 Attestor has been deactivated"
                        );
                    }
                }

                // CASE 9] ATTESTOR FORCE-KICK
                cc_client::attestation::CcEvent::AttestorKicked(attestor) => {
                    if attestor == self.account_id {
                        self.can_attest
                            .store(false, std::sync::atomic::Ordering::Release);
                        tracing::info!(
                            attestor_id = %self.account_id,
                            "💥 Attestor has been kicked"
                        );
                    }
                }

                // CASE 10] ATTESTATION GENESIS BLOCK NUMBER SET
                cc_client::attestation::CcEvent::AttestationChainGenesisBlockNumberSet(
                    _chain_key,
                    genesis_block,
                ) => {
                    tracing::info!(
                        genesis_block,
                        "🎬 Attestation chain genesis block number set"
                    );
                }

                // CASE 11] ATTESTATION CHAIN REVERSION
                cc_client::attestation::CcEvent::RevertedAttestationChainTo(
                    _chain_key,
                    height,
                    digest,
                ) => {
                    let attestation_latest_cc3 = stream::util::AttestationInfo { digest, height };

                    tracing::info!(height, ?digest, "💾 Attestation chain reversion detected!");

                    self.attestation_latest_cc3
                        .store(height, std::sync::atomic::Ordering::Release);
                    self.attestation_local = height;

                    // 1. Update the attestation pool
                    //
                    // Upon chain reversion, we clear all pending attestations in the attestation pool.
                    self.sender_validation
                        .note_attestation_chain_reversion(attestation_latest_cc3);

                    // 2. Update the attestation production stream
                    //
                    // This ensures that we keep producing new attestations starting from the
                    // revert height.
                    self.stream_attestation
                        .note_attestation_chain_reversion(attestation_latest_cc3)
                        .await;
                }
            }
        }

        Ok(())
    }
}
