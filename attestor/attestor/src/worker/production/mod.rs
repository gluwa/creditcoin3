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
//! [attestation pool]: crate::worker::validation::pool
//! [`Attestation`]: crate::common::types::Attestation
//! [p2p worker]: crate::worker::p2p
//! [validation worker]: crate::worker::validation
//! [`Quorum`]: crate::worker::validation::pool::Quorum

mod error;

use crate::prelude::*;
pub use error::*;

// -------------------------------------- [ Configuration ] ------------------------------------ //

/// Attestation production configuration. This includes options to initialize cross-tread
/// communication channels, set up [chain listeners] and store identifying information about an
/// attestor, such as its account id.
#[derive(attestor_macro::Builder)]
pub struct Config {
    stream_attestation: crate::stream::attestation::StreamAttestation,
    stream_cc3: crate::stream::cc3::StreamCC3,

    sender_p2p: tokio::sync::broadcast::Sender<common::types::Attestation>,
    sender_validation: crate::worker::validation::pool::AttestationPoolSender,

    interval_attestation: std::num::NonZero<common::types::Height>,
    attestation_latest_cc3: common::types::AttestationInfo,

    start_height: common::types::Height,
    account_id: cc_client::AccountId32,
    metrics: common::types::Metrics,
}

// ----------------------------------------- [ Worker ] ---------------------------------------- //

pub(crate) struct WorkerAttestationProduction {
    // CHAIN LISTENERS
    stream_attestation: crate::stream::attestation::StreamAttestation,
    stream_cc3: crate::stream::cc3::StreamCC3,

    // MESSAGE CHANNELS
    sender_p2p: tokio::sync::broadcast::Sender<common::types::Attestation>,
    sender_validation: crate::worker::validation::pool::AttestationPoolSender,

    // ATTESTATION DATA
    attestation_local: common::types::Height,
    attestation_latest_cc3: common::types::AttestationInfo,
    attestation_interval: std::num::NonZero<common::types::Height>,

    // METRICS
    metrics: common::types::Metrics,

    // ATTESTOR DATA
    account_id: cc_client::AccountId32,
    can_attest: bool,
}

impl WorkerAttestationProduction {
    pub(crate) fn new(config: Config) -> anyhow::Result<Self> {
        Ok(Self {
            stream_attestation: config.stream_attestation,
            stream_cc3: config.stream_cc3,

            sender_p2p: config.sender_p2p,
            sender_validation: config.sender_validation,

            attestation_local: config.start_height,
            attestation_latest_cc3: config.attestation_latest_cc3,
            attestation_interval: config.interval_attestation,

            metrics: config.metrics,

            account_id: config.account_id,
            can_attest: true,
        })
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
            tokio::select! {
                biased;

                _ = &mut shutdown => {
                    break Err(Interrupt::Stop);
                }
                Some(events) = self.stream_cc3.next() => {
                    self.handle_event_cc3(events).await?;
                }
                Some(event) = self.stream_attestation.next(), if self.can_attest => {
                    self.handle_event_attestation(event).await?;
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
        event: Result<
            crate::stream::attestation::Permit,
            Interrupt<crate::stream::attestation::Error>,
        >,
    ) -> Result<(), Interrupt<Error>> {
        let permit = event.map_interrupt(Error::Attestation)?;
        let now = std::time::Instant::now();

        let attestation = self
            .stream_attestation
            .generate_attestation(permit)
            .await
            .map_interrupt(Error::Attestation)?;

        let height = attestation.header_number();
        let digest = attestation.digest();
        let digest_prev = attestation.prev_digest();
        let attestor_id = attestation.attestor.clone();

        tracing::info!(
            ?digest,
            ?digest_prev,
            height,
            %attestor_id,
            "📡 Generated attestation"
        );

        let attestation_latest_cc3 = self.attestation_latest_cc3.height;

        self.metrics
            .update_attestation_delay_production(now.elapsed());

        self.metrics.set_attestation_local(height);

        self.metrics.update_attestation_lag_eth(
            attestation.header_number(),
            self.stream_attestation.block_highest(),
            self.attestation_interval,
        );
        self.metrics.update_attestation_lag_cc3(
            attestation.header_number(),
            attestation_latest_cc3,
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
        mut events: crate::stream::cc3::StreamEvents,
    ) -> Result<(), Interrupt<Error>> {
        use crate::events::EventAttestationFinalization as _;
        use crate::events::EventAttestationIntervalChange as _;
        use crate::events::EventAttestorsElected as _;
        use crate::events::EventRevertedAttestationChainTo as _;
        use futures::TryStreamExt as _;

        while let Some(event) = events.try_next().await.map_interrupt(Error::CC3)? {
            match event {
                // CASE 1] NEW ATTESTATION
                cc_client::attestation::CcEvent::BlockAttested(attestation) => {
                    let digest = attestation.digest;
                    let height = attestation.header_number;
                    let attestation_latest_cc3 = common::types::AttestationInfo { digest, height };

                    tracing::info!(
                        height,
                        %digest,
                        "💾 New execution chain attestation"
                    );

                    if attestation_latest_cc3.height > self.attestation_latest_cc3.height {
                        self.attestation_latest_cc3 = attestation_latest_cc3;

                        // 1. Chain Listener - Eth
                        //
                        // This is ensure that we keep producing new attestation starting from the
                        // latest finalized on-chain attestation.
                        self.stream_attestation
                            .note_attestation_finalization(attestation_latest_cc3)
                            .expect("Infallible");

                        // 2. Update the attestation pool
                        //
                        // As an edge case, it is possible that we have already generated past
                        // attestations which have not yet been consumed by the validation thread. This
                        // can happen if the production thread is generating attestations faster than
                        // the validation thread can check new quorums. We remove those attestations
                        // here and also update the target block height (if necessary, it is also
                        // possible that we are in advance of the execution chain in which case we do
                        // not want to update the target height and this a no-op).
                        self.sender_validation
                            .note_attestation_finalization(attestation_latest_cc3)
                            .expect("Infallible");

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
                cc_client::attestation::CcEvent::TargetSampleSizeChanged(target_sample_size) => {
                    tracing::info!(target_sample_size, "📏 New target sample size");

                    self.sender_validation
                        .note_target_sample_size_change(target_sample_size);
                }

                // CASE 3] NEW ATTESTATION INTERVAL
                cc_client::attestation::CcEvent::AttestationIntervalChanged(interval) => {
                    tracing::info!(interval, "🔢 New source chain attestation interval");

                    let Some(interval) = std::num::NonZero::<common::types::Height>::new(interval)
                    else {
                        return Ok(());
                    };

                    let attestation_latest_cc3 = self.attestation_latest_cc3.height;

                    // 1. Chain listener - Eth
                    //
                    // Catchup to the new target height and update the attestation interval.
                    self.stream_attestation
                        .note_attestation_interval_change(interval, attestation_latest_cc3)
                        .expect("Infallible");

                    // 2. Attestation pool
                    //
                    // Update quorum validation to expect the new target height and attestation
                    // interval.
                    self.sender_validation
                        .note_attestation_interval_change(interval, attestation_latest_cc3)
                        .expect("Infallible");

                    // 3. Production
                    //
                    // Update local state
                    self.attestation_interval = interval;

                    // 4. Metrics
                    self.metrics.update_attestation_lag_eth(
                        attestation_latest_cc3,
                        self.stream_attestation.block_highest(),
                        interval,
                    );
                    self.metrics.update_attestation_lag_cc3(
                        attestation_latest_cc3,
                        attestation_latest_cc3,
                        interval,
                    );
                }

                cc_client::attestation::CcEvent::CheckpointIntervalChanged(interval) => {
                    tracing::info!(interval, "🔢 New source chain checkpoint interval");
                }

                // CASE 4] NEW ATTESTATION CHECKPOINT
                cc_client::attestation::CcEvent::CheckpointReached(checkpoint) => {
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
                cc_client::attestation::CcEvent::AttestorsElected(attestors) => {
                    tracing::info!("⏰ New attestors elected");

                    // 1. Attestor status
                    //
                    // Update local attestation eligibility.
                    if attestors.contains(&self.account_id) {
                        self.can_attest = true;
                        tracing::info!(
                            account_id = %self.account_id,
                            "☀️ Attestor is eligible for production"
                        );
                    } else {
                        self.can_attest = false;
                        tracing::info!(
                            account_id = %self.account_id,
                            "🛏️ Waiting for attestor to be elected"
                        );
                    }

                    // 2. Attestor validation
                    //
                    // Update the set of legal attestors in the attestation pool.
                    self.sender_validation
                        .note_attestors_elected(attestors)
                        .expect("Infallible");
                }

                // CASE 7] ATTESTOR ACTIVATION
                cc_client::attestation::CcEvent::AttestorActivated(attestor) => {
                    if attestor == self.account_id {
                        tracing::info!(
                            account_id = %self.account_id,
                            "🔋 Attestor has been activated"
                        );
                    }
                }

                // CASE 8] ATTESTOR DEACTIVATION
                cc_client::attestation::CcEvent::AttestorChilled(attestor) => {
                    if attestor == self.account_id {
                        self.can_attest = false;
                        tracing::info!(
                            account_id = %self.account_id,
                            "🪫 Attestor has been deactivated"
                        );
                    }
                }

                // CASE 9] ATTESTOR FORCE-KICK
                cc_client::attestation::CcEvent::AttestorKicked(attestor) => {
                    if attestor == self.account_id {
                        self.can_attest = false;
                        tracing::info!(
                            account_id = %self.account_id,
                            "💥 Attestor has been kicked"
                        );
                    }
                }

                // CASE 10] ATTESTATION GENESIS BLOCK NUMBER SET
                cc_client::attestation::CcEvent::AttestationChainGenesisBlockNumberSet(
                    genesis_block,
                ) => {
                    tracing::info!(
                        genesis_block,
                        "🎬 Attestation chain genesis block number set"
                    );
                }
            }
        }

        Ok(())
    }
}
