#![doc = include_str!("../../../../mermaid.html")]
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
//!   execution chain, invalidating all local attestation prior to it which have not yet been
//!   submitted to the runtime.
//!
//! - **[`RandomnessChanged`]**: New execution chain epoch, implies a rotation in the validator set
//!   and an invalidation of all local non-finalized attestations.
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
    eth: crate::chain_listener::eth::Ethereum,
    cc3: crate::chain_listener::cc3::CC3,

    rebroadcast: crate::chain_listener::rebroadcast::Rebroadcast,
    sender_p2p: tokio::sync::broadcast::Sender<common::types::Attestation>,
    sender_validation: crate::worker::validation::pool::AttestationPoolSender,
    sender_attestation_latest: tokio::sync::watch::Sender<Option<common::types::Height>>,

    attestation_start_cc3: Option<(attestor_primitives::Digest, common::types::Height)>,
    epoch: common::types::Epoch,

    account_id: cc_client::AccountId32,

    can_broadcast: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

// ----------------------------------------- [ Worker ] ---------------------------------------- //

pub(crate) struct WorkerAttestationProduction {
    // CHAIN LISTENERS
    eth: crate::chain_listener::eth::Ethereum,
    cc3: crate::chain_listener::cc3::CC3,
    rebroadcast: crate::chain_listener::rebroadcast::Rebroadcast,

    // MESSAGE CHANNELS
    sender_p2p: tokio::sync::broadcast::Sender<common::types::Attestation>,
    sender_validation: crate::worker::validation::pool::AttestationPoolSender,
    sender_attestation_latest: tokio::sync::watch::Sender<Option<common::types::Height>>,

    // CHAIN DATA
    attestation_latest_eth: Option<(attestor_primitives::Digest, common::types::Height)>,
    attestation_latest_cc3: Option<(attestor_primitives::Digest, common::types::Height)>,
    epoch: common::types::Epoch,

    // ATTESTOR DATA
    account_id: cc_client::AccountId32,
    can_attest: bool,

    // P2P DATA
    can_broadcast: std::sync::Arc<std::sync::atomic::AtomicBool>,

    // ATTESTATION CACHE
    attestations: std::collections::HashMap<common::types::Height, common::types::Attestation>,
}

impl WorkerAttestationProduction {
    pub(crate) async fn new(config: Config) -> common::types::Result<Self> {
        let can_attest = config.cc3.can_attest().await?;

        Ok(Self {
            eth: config.eth,
            cc3: config.cc3,
            rebroadcast: config.rebroadcast,

            sender_p2p: config.sender_p2p,
            sender_validation: config.sender_validation,
            sender_attestation_latest: config.sender_attestation_latest,

            attestation_latest_eth: None,
            attestation_latest_cc3: config.attestation_start_cc3,
            epoch: config.epoch,

            account_id: config.account_id,
            can_attest,

            can_broadcast: config.can_broadcast,

            attestations: std::collections::HashMap::new(),
        })
    }
}

// ---------------------------------------- [ Main loop ] -------------------------------------- //

impl super::Worker for WorkerAttestationProduction {
    #[tracing::instrument(name = "production", skip_all)]
    fn task(
        mut self,
        mut shutdown: std::pin::Pin<Box<impl std::future::Future<Output = ()>>>,
    ) -> impl std::future::Future<Output = common::types::Result<()>> {
        async move {
            loop {
                let can_broadcast = self
                    .can_broadcast
                    .load(std::sync::atomic::Ordering::Acquire);

                tokio::select! {
                    biased;

                    _ = &mut shutdown => {
                        break self.handle_event_shutdown().await;
                    }
                    Some(event) = self.cc3.next() => {
                        self.handle_event_cc3(event).await?;
                    }
                    Some(event) = self.rebroadcast.next(), if self.can_attest && can_broadcast => {
                        self.handle_event_rebroadcast(event).await?;
                    }
                    Some(event) = self.eth.next(), if self.can_attest => {
                        self.handle_event_eth(event).await?;
                    }
                }
            }
        }
    }
}

// ----------------------------------------- [ Events ] ---------------------------------------- //

impl WorkerAttestationProduction {
    // ----------------------------------------* Eth events *--------------------------------------

    async fn handle_event_eth(
        &mut self,
        event: Result<crate::common::types::Height, crate::chain_listener::eth::Error>,
    ) -> Result<(), Error> {
        // STEP 1] GENERATE CONTINUITY PROOF

        let height = event.map_err(Error::EthError)?;

        tracing::debug!(height, "Generating attestation");

        let continuity_fragment = match self
            .cc3
            .create_continuity_proof(
                height,
                self.attestation_latest_eth,
                self.attestation_latest_cc3,
            )
            .await
        {
            Some(Ok(continuity_fragment)) => continuity_fragment,
            Some(Err(err)) => return Err(Error::CC3Error(err)),
            None => return Ok(()),
        };

        // STEP 2] GENERATE ATTESTATION

        let block = self.eth.get_block(height).await.map_err(Error::EthError)?;
        let prev_digest = continuity_fragment.head().map(|head| head.digest);

        let attestation = attestor_primitives::AttestationData::<attestor_primitives::Digest>::new(
            self.cc3.get_chain_key(),
            block.number(),
            sp_core::H256(*block.hash()),
            eth::simple_merkle_tree(&block).root(),
            prev_digest,
        );

        let attestation_signed = self
            .cc3
            .sign_attestation(attestation, continuity_fragment, self.epoch)
            .await;

        // STEP 3] BROADCAST ATTESTATION

        let digest = attestation_signed.digest();
        let digest_prev = attestation_signed.prev_digest();
        let attestor_id = &attestation_signed.attestor;
        tracing::info!(
            %digest,
            ?digest_prev,
            height,
            %attestor_id,
            "📡 Generated attestation"
        );

        // From the tokio docs:
        //
        // > A send operation can only fail if there are no active receivers, implying that the
        // > message could never be received.
        //
        // This can happen during shutdown and does not represent a failing case!
        _ = self.sender_p2p.send(attestation_signed.clone());

        tracing::info!(
            %digest,
            height,
            %attestor_id,
            "🗳️ Sending local attestation over for validation"
        );

        // STEP 4] STORE ATTESTATION

        assert!(
            self.attestations
                .insert(height, attestation_signed.clone())
                .is_none(),
            "Invariant violated: regenerating existing attestation"
        );

        if let Err(err) = self.sender_validation.send(attestation_signed) {
            err.log_error(digest);
        }

        // STEP 5] UPDATE SYNC STATUS

        self.attestation_latest_eth = Some((digest, height));
        self.rebroadcast.note_attestation_production(height);

        Ok(())
    }

    // ----------------------------------------* CC3 events *--------------------------------------

    async fn handle_event_cc3(
        &mut self,
        res: Result<crate::chain_listener::cc3::CC3Events, crate::chain_listener::cc3::Error>,
    ) -> Result<(), Error> {
        for event in res
            .map_err(Error::CC3Error)?
            .events()
            .await
            .map_err(Error::CC3Error)?
        {
            match event.map_err(Error::CC3Error)? {
                // CASE 1] NEW ATTESTATION
                cc_client::attestation::CcEvent::BlockAttested(attestation) => {
                    if attestation.chain_key() == self.cc3.get_chain_key() {
                        let digest = attestation.digest();
                        let height = attestation.header_number();

                        tracing::info!(
                            height,
                            %digest,
                            "💾 New execution chain attestation"
                        );

                        self.attestation_latest_cc3 = Some((digest, height));

                        // 1. Chain Listener - Eth
                        //
                        // This is ensure that we keep producing new attestation starting from the
                        // latest finalized on-chain attestation.
                        self.eth.note_attestation_finalization(height);

                        // 2. Chain Listener - Rebroadcast
                        //
                        // Makes it so we do not re-generate attestations which have already been
                        // finalized on-chain (it is still possible for a race condition to occur where
                        // we would re-submit a past attestation before noticing the `BlockAttested`
                        // event, but that is handled as a non-failure case by the validation worker).
                        self.rebroadcast.note_attestation_finalization(height);

                        // 3. Update the attestation pool
                        //
                        // As an edge case, it is possible that we have already generated past
                        // attestations which have not yet been consumed by the validation thread. This
                        // can happen if the production thread is generating attestations faster than
                        // the validation thread can check new quorums. We remove those attestations
                        // here and also update the target block height (if necessary, it is also
                        // possible that we are in advance of the execution chain in which case we do
                        // not want to update the target height and this a no-op).
                        self.sender_validation.note_attestation_finalization(height);

                        // 4. Notify the validation worker
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

                        // 5. Production
                        //
                        // Clear local state
                        self.attestations.retain(|h, _att| *h > height);
                    }
                }

                // CASE 2] NEW ATTESTATION INTERVAL
                cc_client::attestation::CcEvent::AttestationIntervalChanged(
                    chain_key,
                    interval,
                ) => {
                    if chain_key == self.cc3.get_chain_key() {
                        tracing::info!(interval, "🔢 New source chain attestation interval");

                        let interval = std::num::NonZero::<common::types::Height>::new(interval)
                            .unwrap_or(std::num::NonZero::<common::types::Height>::MIN);

                        let attestation_latest_cc3 = self
                            .attestation_latest_cc3
                            .as_ref()
                            .map(|(_digest, height)| *height);

                        // 1. Chain listener - Eth
                        //
                        // Catchup to the new target height and update the attestation interval.
                        self.eth
                            .note_attestation_interval_change(interval, attestation_latest_cc3)
                            .await
                            .map_err(Error::EthError)?;

                        // 2. Attestation pool
                        //
                        // Update quorum validation to expect the new target height and attestation
                        // interval.
                        self.sender_validation
                            .note_attestation_interval_change(interval, attestation_latest_cc3);

                        // 3. Production
                        //
                        // Clear local state
                        self.attestation_latest_eth = None;
                        self.attestations.clear();
                    }
                }

                // CASE 3] NEW ATTESTATION CHECKPOINT
                cc_client::attestation::CcEvent::CheckpointReached(chain_key, checkpoint) => {
                    if chain_key == self.cc3.get_chain_key() {
                        tracing::info!(
                            height = checkpoint.block_number,
                            digest = ?checkpoint.digest,
                            "🛟 New execution chain attestation checkpoint"
                        )
                    }
                }

                // CASE 4] NEW EPOCH
                cc_client::attestation::CcEvent::RandomnessChanged((epoch, _randomness)) => {
                    tracing::info!(epoch, "🎲 New epoch rotation");
                    self.epoch = epoch;
                }

                // CASE 5] ATTESTOR ELECTION
                cc_client::attestation::CcEvent::AttestorsElected(chain_key, attestors) => {
                    if chain_key == self.cc3.get_chain_key() {
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
                        let _ = self.sender_validation.note_attestors_elected(attestors);
                    }
                }

                // CASE 6] ATTESTOR ACTIVATION
                cc_client::attestation::CcEvent::AttestorActivated(attestor) => {
                    if attestor == self.account_id {
                        tracing::info!(
                            account_id = %self.account_id,
                            "🔋 Attestor has been activated"
                        );
                    }
                }

                // CASE 7] ATTESTOR DEACTIVATION
                cc_client::attestation::CcEvent::AttestorChilled(attestor) => {
                    if attestor == self.account_id {
                        self.can_attest = false;
                        tracing::info!(
                            account_id = %self.account_id,
                            "🪫 Attestor has been deactivated"
                        );
                    }
                }

                // CASE 8] ATTESTOR FORCE-KICK
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

    // ---------------------------------------* Rebroadcast *--------------------------------------

    async fn handle_event_rebroadcast(&mut self, height: u64) -> Result<(), Error> {
        // NOTE: The rebroadcast listener is not aware of each attestation in local storage and
        // instead sends us a contiguous range of block heights to rebroadcast. Quite frankly this
        // makes the rebroadcasting logic easier, but it does mean we might not have all the
        // requested attestations available to send.
        if let Some(attestation) = self.attestations.get(&height) {
            // WARNING: ERROR HANDLING
            //
            // From the tokio docs:
            //
            // > A send operation can only fail if there are no active receivers, implying that the
            // > message could never be received.
            //
            // This can happen during shutdown and does not represent a failing case!
            _ = self.sender_p2p.send(attestation.clone());
        }

        Ok(())
    }

    // -----------------------------------------* Shutdown *---------------------------------------

    async fn handle_event_shutdown(&mut self) -> common::types::Result<()> {
        Ok(())
    }
}
