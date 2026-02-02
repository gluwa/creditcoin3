//! A [`Worker`] thread responsible for the production of new attestations.
//!
//! # Ethereum Data
//!
//! The production worker keeps track of source chain finality via the [eth chain listener], which
//! abstracts away a of lot the complexity associated with the synchronization of new source chain
//! blocks. When a new source chain block is noticed and if it is past the
//! [`ATTESTATION_FINALIZATION_LAG`], this triggers the production of a new [`Attestation`].
//!
//! Attestation computation occurs in two steps:
//!
//! 1. **Continuity fragment computation**: This is a blocking operation, and is parallelized across
//!    a [`rayon`] thread pool to speed up computation.
//! 2. **Attestation signing**: The generated attestation is signed with an attestor's private BLS
//!    key to guarantee authenticity and integrity. Attestation BLS signatures are later aggregated
//!    on submision by the [validation worker] to prove [`Quorum`].
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
//! events. These events are then forwarded to the [eth chain listener], [cc3 chain listener] and
//! the [attestation pool] for further handling. The following events are supported.
//!
//! - **[`BlockAttested`]**: A new source chain attestation has been finalized on the execution
//!   chain, invalidating all local attestation prior to it which have not yet been submitted to the
//!   runtime.
//!
//! - **[`AttestationIntervalChanged`]**: New attestation production interval. This causes all
//!   local attestations to be invalidated as we set the target attestation height to be the next
//!   closest multiple of the new attestation interval.
//!
//! - **[`CheckpointReached`]**: A new source chain attestation checkpoint has been finalized on the
//!   execution chain.
//!
//! - **[`RandomnessChanged`]**: New execution chain epoch, implies a rotation in the validator
//!   set, which is emitted as a separate event.
//!
//! - **[`AttestorsElected`]**: Rotation in the elected attestor set. This lets us know which
//!   attestors are allowed to submit attestations for the coming epoch.
//!
//! - **[`AttestorActivated`]**: Attestor registration. Keep in mind that an attestor still has to
//!   wait for the next rotation in the elected attestor set to see if it is eligible to start
//!   producing attestations.
//!
//! - **[`AttestorChilled`]**: Attestor deactivation. Attestors need to be chilled before they can
//!   be un-registered, indicating they will no longer produce new attestations.
//!
//! [`Worker`]: crate::worker::Worker
//! [eth chain listener]: crate::chain_listener::eth
//! [cc3 chain listener]: crate::chain_listener::cc3
//! [attestation pool]: crate::worker::validation::pool
//! [`ATTESTATION_FINALIZATION_LAG`]: crate::common::constants::ATTESTATION_FINALIZATION_LAG
//! [`Attestation`]: crate::common::types::Attestation
//! [p2p worker]: crate::worker::p2p
//! [validation worker]: crate::worker::validation
//! [`Quorum`]: crate::worker::validation::pool::Quorum
//! [`BlockAttested`]: cc_client::attestation::CcEvent::BlockAttested
//! [`AttestationIntervalChanged`]: cc_client::attestation::CcEvent::AttestationIntervalChanged
//! [`CheckpointReached`]: cc_client::attestation::CcEvent::CheckpointReached
//! [`RandomnessChanged`]: cc_client::attestation::CcEvent::RandomnessChanged
//! [`AttestorsElected`]: cc_client::attestation::CcEvent::AttestorsElected
//! [`AttestorActivated`]: cc_client::attestation::CcEvent::AttestorActivated
//! [`AttestorChilled`]: cc_client::attestation::CcEvent::AttestorChilled

mod error;
mod stream;

use crate::prelude::*;
pub use error::*;

// -------------------------------------- [ Configuration ] ------------------------------------ //

/// Attestation production configuration. This includes options to initialize cross-tread
/// communication channels, set up [chain listeners] and store identifying information about an
/// attestor, such as its account id.
///
/// [chain listeners]: crate::chain_listener
#[derive(attestor_macro::Builder)]
pub struct Config {
    eth: eth::Client,
    cc3: cc_client::Client,

    sender_p2p: tokio::sync::broadcast::Sender<common::types::Attestation>,
    sender_validation: crate::worker::validation::pool::AttestationPoolSender,
    sender_attestation_latest: tokio::sync::watch::Sender<Option<common::types::Height>>,

    interval_attestation: std::num::NonZero<common::types::Height>,
    interval_checkpoint: std::num::NonZero<common::types::Height>,

    chain_key: attestor_primitives::ChainKey,
    start_height: common::types::Height,
    start_digest: Option<attestor_primitives::Digest>,
    empty_chain: bool,

    account_id: cc_client::AccountId32,
    bls_key: bls_signatures::PrivateKey,

    metrics: common::types::Metrics,
}

// ----------------------------------------- [ Worker ] ---------------------------------------- //

pub(crate) struct WorkerAttestationProduction {
    // CHAIN LISTENERS
    stream_attestation: stream::attestation::StreamAttestation,
    stream_cc3: stream::cc3::StreamCC3,

    // MESSAGE CHANNELS
    sender_p2p: tokio::sync::broadcast::Sender<common::types::Attestation>,
    sender_validation: crate::worker::validation::pool::AttestationPoolSender,
    sender_attestation_latest: tokio::sync::watch::Sender<Option<common::types::Height>>,

    // ATTESTATION DATA
    attestation_local: common::types::Height,
    attestation_latest_cc3: common::types::AttestationInfo,
    attestation_interval: std::num::NonZero<common::types::Height>,

    // CHAIN DATA
    start_height: common::types::Height,
    chain_key: attestor_primitives::ChainKey,

    // METRICS
    metrics: common::types::Metrics,

    // ATTESTOR DATA
    account_id: cc_client::AccountId32,
    can_attest: bool,
}

impl WorkerAttestationProduction {
    pub(crate) async fn new(config: Config) -> common::types::Result<Self> {
        use anyhow::Context as _;
        use futures::StreamExt as _;

        let mut stream_attestation = stream::attestation::StreamAttestation::new(
            stream::attestation::ConfigBuilder::new()
                .with_cc3(config.cc3.clone())
                .with_eth(config.eth)
                .with_bls_key(config.bls_key)
                .with_interval_attestation(config.interval_attestation)
                .with_interval_checkpoint(config.interval_checkpoint)
                .with_chain_key(config.chain_key)
                .with_start_height(config.start_height)
                .with_start_digest(config.start_digest)
                .build(),
        )
        .await
        .context("Failed to create attestation stream")?;

        let mut stream_cc3 = stream::cc3::StreamCC3::new(
            stream::cc3::ConfigBuilder::new()
                .with_cc3(config.cc3.clone())
                .with_chain_key(config.chain_key)
                .build(),
        )
        .await
        .context("Failed to create cc3 events stream")?;

        tracing::info!(
            attestor = %config.account_id,
            "⏲️ Waiting for attestor to be made eligible"
        );

        let can_attest = config
            .cc3
            .get_attestor_status(config.chain_key)
            .await
            .context("Failed to retrieve attestor status")?
            .as_ref()
            .is_some_and(attestor_primitives::AttestorStatus::is_active);

        if !can_attest {
            for block in stream_cc3
                .next()
                .await
                .context("Failed to retrieve events")?
            {
                for event in block.events().await.map_err(Error::CC3)? {
                    let event = event.map_err(Error::CC3)?;
                    if let cc_client::attestation::CcEvent::AttestorsElected(attestors) = event {
                        if attestors.contains(&config.account_id) {
                            break;
                        }
                    }
                }
            }
        }

        tracing::info!(
            height = config.start_height,
            "👶 Generating initial attestation"
        );

        let attestation = if config.empty_chain {
            stream_attestation
                .generate_attestation_genesis()
                .await
                .context("Failed to generate genesis attestation")?
        } else {
            let permit = stream_attestation
                .next()
                .await
                .context("Unexpected end of stream")?
                .context("Failed to generate attestation")?;
            stream_attestation
                .generate_attestation(permit)
                .await
                .context("Failed to generate attestation")?
        };

        let height = attestation.header_number();
        let digest = attestation.digest();
        let digest_prev = attestation.prev_digest();
        let attestor_id = attestation.attestor.clone();

        tracing::info!(
            ?digest,
            ?digest_prev,
            height,
            %attestor_id,
            "📡 Generated intial attestation"
        );

        config
            .sender_p2p
            .send(attestation.clone())
            .context("Failed to send initial attestation over to p2p worker")?;
        config
            .sender_validation
            .send(attestation)
            .transpose()
            .context("Failed to send initial attestation over for validation")?;

        tracing::info!(
            ?digest,
            ?digest_prev,
            height,
            %attestor_id,
            "⏲️ Waiting for intial attestation to finalize"
        );

        loop {
            let block = stream_cc3
                .next()
                .await
                .context("Unexpected end of stream")?
                .context("Failed to retrieve events")?;

            for event in block.events().await.map_err(Error::CC3)? {
                let event = event.map_err(Error::CC3)?;
                if let cc_client::attestation::CcEvent::BlockAttested(attestation) = event {
                    if attestation.header_number >= height {
                        let attestation_latest = common::types::AttestationInfo {
                            digest: attestation.digest,
                            height: attestation.header_number,
                        };

                        return Ok(Self {
                            stream_attestation,
                            stream_cc3,

                            sender_p2p: config.sender_p2p,
                            sender_validation: config.sender_validation,
                            sender_attestation_latest: config.sender_attestation_latest,

                            attestation_local: height,
                            attestation_latest_cc3: attestation_latest,
                            attestation_interval: config.interval_attestation,

                            start_height: config.start_height,
                            chain_key: config.chain_key,

                            metrics: config.metrics,

                            account_id: config.account_id,
                            can_attest: true,
                        });
                    }
                }
            }
        }
    }
}

// ---------------------------------------- [ Main loop ] -------------------------------------- //

impl super::Worker for WorkerAttestationProduction {
    #[tracing::instrument(name = "production", skip_all)]
    async fn task(
        mut self,
        mut shutdown: std::pin::Pin<Box<impl std::future::Future<Output = ()>>>,
    ) -> common::types::Result<()> {
        use futures::StreamExt as _;

        loop {
            tokio::select! {
                biased;

                _ = &mut shutdown => {
                    break self.handle_event_shutdown().await;
                }
                Some(event) = self.stream_cc3.next() => {
                    self.handle_event_cc3(event).await?;
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
        event: Result<stream::attestation::Permit, stream::attestation::Error>,
    ) -> Result<(), Error> {
        let permit = event.map_err(Error::Attestation)?;
        let now = std::time::Instant::now();

        let attestation = self
            .stream_attestation
            .generate_attestation(permit)
            .await
            .map_err(Error::Attestation)?;

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

        let attestation_latest = self.attestation_latest_cc3.height;

        self.metrics
            .update_attestation_delay_production(now.elapsed());

        self.metrics
            .set_attestation_local(attestation.header_number());

        self.metrics.update_attestation_lag_eth(
            attestation.header_number(),
            self.stream_attestation.block_highest(),
            self.attestation_interval,
        );
        self.metrics.update_attestation_lag_cc3(
            attestation.header_number(),
            attestation_latest,
            self.attestation_interval,
        );

        tracing::info!(
            ?digest,
            attestation_latest,
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

        self.attestation_local = attestation_latest;

        Ok(())
    }

    // ----------------------------------------* CC3 events *--------------------------------------

    async fn handle_event_cc3(
        &mut self,
        res: Result<stream::cc3::CC3Events, stream::cc3::Error>,
    ) -> Result<(), Error> {
        use crate::events::EventAttestationFinalization as _;
        use crate::events::EventAttestationIntervalChange as _;
        use crate::events::EventAttestationIntervalChangeAsync as _;
        use crate::events::EventAttestorsElected as _;
        use crate::events::EventCheckpointIntervalChange as _;

        for event in res
            .map_err(Error::CC3)?
            .events()
            .await
            .map_err(Error::CC3)?
        {
            match event.map_err(Error::CC3)? {
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

                    // 3. Notify the validation worker
                    //
                    // This lets the validation worker know it can start submitting attestations at
                    // a greater height, if it has any.
                    //
                    // WARNING: ERROR HANDLING
                    //
                    // From the tokio docs:
                    //
                    // > This method fails if the channel is closed, which is the case when every
                    // > receiver has been dropped.
                    //
                    // This only errors if the receiving end of this channel has been dropped, which
                    // can happen during shutdown. This is not a failure case!
                    let _ = self.sender_attestation_latest.send(Some(height));

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

                // CASE 2] NEW TARGET SAMPLE SIZE
                cc_client::attestation::CcEvent::TargetSampleSizeChanged(target_sample_size) => {
                    tracing::info!(target_sample_size, "📏 New target sample size");

                    self.sender_validation
                        .note_target_sample_size_change(target_sample_size);
                }

                // CASE 2] NEW ATTESTATION INTERVAL
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

                    let Some(interval) = std::num::NonZero::<common::types::Height>::new(interval)
                    else {
                        return Ok(());
                    };

                    let attestation_latest_cc3 = self.attestation_latest_cc3.height;

                    todo!()
                }

                // CASE 3] NEW ATTESTATION CHECKPOINT
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

                // CASE 5] ATTESTOR ELECTION
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
            }
        }

        Ok(())
    }

    // -----------------------------------------* Shutdown *---------------------------------------

    async fn handle_event_shutdown(&mut self) -> common::types::Result<()> {
        Ok(())
    }
}
