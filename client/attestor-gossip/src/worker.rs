use futures::StreamExt;
use log::{debug, error, info, warn};
use parity_scale_codec::{Codec, Decode, Encode};
use sc_client_api::{Backend, BlockBackend, FinalityNotification, HeaderBackend};
use sc_utils::mpsc::TracingUnboundedReceiver;
use sp_api::ProvideRuntimeApi;
use sp_consensus_babe::BabeApi;
use sp_core::H256;
use sp_inherents::CreateInherentDataProviders;
use sp_runtime::traits::{Block as BlockT, Hash as HashT, Header as HeaderT};
use std::fmt::{Debug, Display};
use std::marker::PhantomData;
use std::sync::Arc;

use attestor_primitives::{
    api::AttestorApi,
    bls::{Bls, CryptoScheme, WrapEncode},
    Round, SignedAttestation,
};

use bls_signatures::{aggregate, Serialize};
use randomness_primitives::api::RandomnessPalletApi;
use supported_chains_primitives::api::SupportedChainsApi;

use super::{inherent, AttestorComms, Client, Error, HashFor, Message, LOG_TARGET};
use crate::communication::Attestation;
use crate::round;
use crate::state::{State, VoteImportResult};

/// Gossip engine votes messages topic
pub(crate) fn votes_topic<B: BlockT>() -> B::Hash
where
    B: BlockT,
{
    <<B::Header as HeaderT>::Hashing as HashT>::hash(b"attestor-votes")
}

pub(crate) struct Worker<B: BlockT, RuntimeApi: ProvideRuntimeApi<B>, BE, C, CIDP, AccountId>
where
    RuntimeApi: ProvideRuntimeApi<B> + Send + Sync + 'static,
    RuntimeApi::Api: BabeApi<B>,
    RuntimeApi::Api: AttestorApi<B, HashFor<B>, AccountId>,
    RuntimeApi::Api: SupportedChainsApi<B>,
    RuntimeApi::Api: RandomnessPalletApi<B>,
    BE: Backend<B> + 'static,
    AccountId: Clone
        + Display
        + Codec
        + Send
        + 'static
        + Sync
        + Debug
        + Into<[u8; 32]>
        + PartialEq
        + Eq
        + std::hash::Hash,
{
    /// communication (created once, but returned and reused if worker is restarted/reinitialized)
    pub comms: AttestorComms<B, AccountId, RuntimeApi, BE>,

    /// runtime api access
    pub runtime: Arc<RuntimeApi>,

    #[allow(dead_code)]
    pub client: Arc<C>,

    /// Client Backend
    pub backend: Arc<BE>,

    pub state: State<HashFor<B>, AccountId>,

    /// Current epoch index
    pub current_epoch_index: u64,

    /// Inherent data providers
    #[allow(dead_code)]
    pub create_inherent_data_providers: CIDP,

    pub inherent_provider: inherent::AsyncProvider<AccountId, B, RuntimeApi, BE>,

    pub _phantom: PhantomData<AccountId>,
}

pub(crate) struct WorkerParams<B: BlockT, RuntimeApi: ProvideRuntimeApi<B>, BE, C, CIDP, AccountId>
where
    RuntimeApi: ProvideRuntimeApi<B> + Send + Sync + 'static,
    RuntimeApi::Api: BabeApi<B>,
    RuntimeApi::Api: AttestorApi<B, HashFor<B>, AccountId>,
    RuntimeApi::Api: SupportedChainsApi<B>,
    RuntimeApi::Api: RandomnessPalletApi<B>,
    BE: Backend<B> + 'static,
    AccountId: Clone
        + Display
        + Codec
        + Send
        + 'static
        + Sync
        + Debug
        + Into<[u8; 32]>
        + PartialEq
        + Eq
        + std::hash::Hash,
{
    /// communication (created once, but returned and reused if worker is restarted/reinitialized)
    pub comms: AttestorComms<B, AccountId, RuntimeApi, BE>,

    /// runtime api access
    pub runtime: Arc<RuntimeApi>,

    pub client: Arc<C>,

    /// Inherent data providers
    pub create_inherent_data_providers: CIDP,

    /// Client Backend
    pub backend: Arc<BE>,

    pub inherent_provider: inherent::AsyncProvider<AccountId, B, RuntimeApi, BE>,

    pub _phantom: PhantomData<AccountId>,
}

impl<B: BlockT, RA: ProvideRuntimeApi<B>, BE, C, CIDP, AccountId>
    Worker<B, RA, BE, C, CIDP, AccountId>
where
    B: BlockT,
    RA: ProvideRuntimeApi<B> + Send + Sync + 'static,
    RA::Api: BabeApi<B>,
    RA::Api: AttestorApi<B, HashFor<B>, AccountId>,
    RA::Api: SupportedChainsApi<B>,
    RA::Api: RandomnessPalletApi<B>,
    BE: Backend<B>,
    C: Client<B, BE> + BlockBackend<B>,
    CIDP: CreateInherentDataProviders<B, ()> + 'static,
    H256: From<<B as BlockT>::Hash>,
    <B as BlockT>::Hash: From<H256>,
    <<B as BlockT>::Header as HeaderT>::Number: Into<u64>,
    AccountId: Clone
        + Display
        + Codec
        + Send
        + 'static
        + Sync
        + Debug
        + Into<[u8; 32]>
        + PartialEq
        + Eq
        + std::hash::Hash,
{
    pub fn new(params: WorkerParams<B, RA, BE, C, CIDP, AccountId>) -> Self {
        let block_hash = params.backend.blockchain().info().finalized_hash;

        let current_epoch_index = params
            .runtime
            .runtime_api()
            .current_epoch(block_hash)
            .expect("Failed to get current epoch index");

        Worker {
            comms: params.comms,
            runtime: params.runtime,
            client: params.client,
            create_inherent_data_providers: params.create_inherent_data_providers,
            state: State::default(),
            current_epoch_index: current_epoch_index.epoch_index,
            backend: params.backend,
            inherent_provider: params.inherent_provider,
            _phantom: PhantomData,
        }
    }

    pub async fn start(
        mut self,
        mut finality_notifications: TracingUnboundedReceiver<FinalityNotification<B>>,
    ) -> Error {
        let mut votes = Box::pin(
            self.comms
                .gossip_engine
                .messages_for(votes_topic::<B>())
                .filter_map(|notification| async move {
                    Message::<B, AccountId>::decode(&mut &notification.message[..]).ok()
                })
                .fuse(),
        );

        // Main process loop
        loop {
            // Mutable reference used to drive the gossip engine.
            let mut gossip_engine = &mut self.comms.gossip_engine;
            let message_stream = &mut self.comms.gossip_report_stream;

            futures::select_biased! {
                // Use `select_biased!` to prioritize order below.
                // Process finality notifications first since these drive the voter.
                notification = finality_notifications.next() => {
                    if let Some(notif) = notification {
                        if let Err(err) = self.handle_finality_notification(&notif) {
                            break err;
                        }
                    } else {
                        break Error::FinalityStreamTerminated;
                    }
                },
                // Make sure to pump gossip engine.
                _ = gossip_engine => {
                    break Error::GossipEngineExited;
                },
                // Handler that handles incoming attestation from the gossip netowrk
                vote = votes.next() => {
                    if let Some(vote) = vote {
                        debug!(target: LOG_TARGET, "📝 Got a vote from the network");
                        match self.triage_message(vote.clone()).await {
                            Ok(()) => {
                                debug!(target: LOG_TARGET, "📝 Got a valid gossiped message");
                            },
                            Err(e) => {
                                info!(target: LOG_TARGET, "📝 Got error for message err: {:?}", e);
                            }
                        }
                    } else {
                        warn!(target: LOG_TARGET, "📝 Got a vote, but it was invalid");
                        break Error::GossipEngineExited;
                    }
                },
                // Handler that handles incoming attestation from it's rpc endpoint
                // This is the main entry point for the attestation worker
                // It will handle incoming attestations, and gossip them to the network
                message = message_stream.next() => {
                    if let Some(message) = message {
                        let topic = votes_topic::<B>();

                        match message.clone() {
                            Message::Attestation(attestation) => {
                                let chain_key = attestation.attestation_data.chain_key;
                                let header_number = attestation.attestation_data.header_number;

                                let round = (chain_key, header_number);
                                debug!(target: LOG_TARGET, "📝 Got attestation to gossip with digest {:?}, on topic: {:?} for round {:?}", attestation.digest(), topic, round);

                                // Gossip to peers first
                                gossip_engine.gossip_message(
                                    topic,
                                    message.encode(),
                                    true,
                                );

                                // Also process the message
                                match self.process_attestation_message(attestation).await {
                                    Ok(()) => {
                                        info!(target: LOG_TARGET, "📝 Got a valid incoming message from rpc");
                                    },
                                    Err(e) => {
                                        info!(target: LOG_TARGET, "📝 Got error for message err: {:?}", e);
                                    }
                                }
                            },
                        };
                    }
                }
            }
        }
    }

    /// Triage incoming messages
    /// This function is responsible for deciding what to do with incoming messages
    async fn triage_message(&mut self, message: Message<B, AccountId>) -> Result<(), Error> {
        match message {
            Message::Attestation(attestation) => {
                self.process_attestation_message(attestation).await?;
            }
        }

        Ok(())
    }

    async fn process_attestation_message(
        &mut self,
        attestation: Attestation<HashFor<B>, AccountId>,
    ) -> Result<(), Error> {
        let block_hash = self.backend.blockchain().info().best_hash;

        // Get the round for the attestation
        // This is the chain key and header number
        let round = attestation.round();

        // Verify the VRF output
        self.verify_vrf(round, &attestation)?;

        let round_config =
            round::get_round_config(self.runtime.clone(), round.0, block_hash, &self.state)?;

        // Get the current epoch
        let current_epoch = self
            .runtime
            .runtime_api()
            .current_epoch(block_hash)?
            .epoch_index;
        // Add the attestation to the round
        let import_result =
            self.state
                .note_vote(attestation.clone(), &round_config, current_epoch)?;

        match import_result {
            VoteImportResult::DoubleVote => {
                warn!(target: LOG_TARGET, "📝 Double vote detected");
            }
            VoteImportResult::Ok => {
                info!(target: LOG_TARGET, "📝 Attestation added to round");
            }
            VoteImportResult::RoundConcluded => {
                info!(target: LOG_TARGET, "📝 Round concluded");
                self.try_submit_attestation(round, attestation)?;
            }
        }

        Ok(())
    }

    /// Verify the VRF output for an attestation.
    /// This checks if the attestor that submitted this attestations vrf output is correct
    /// Correct being, that it signed the babe's VRF output from Two epochs ago & that the attestor is eligible to submit an attestation
    fn verify_vrf(
        &self,
        round: Round,
        attestation: &Attestation<HashFor<B>, AccountId>,
    ) -> Result<(), Error> {
        info!(target: LOG_TARGET, "📝 Verifying VRF output for attestation, round: {:?}", round);
        let best_hash = self.backend.blockchain().info().finalized_hash;
        let chain_key = round.0;
        let header_number = round.1;

        let current_epoch = self
            .runtime
            .runtime_api()
            .current_epoch(best_hash)?
            .epoch_index;

        // Now check if the attestor was valid in the epoch that it tells us it's attesting for
        let is_valid_attestor = self.state.check_chain_attestor_epoch_inclusion(
            chain_key,
            current_epoch,
            attestation.attestor.clone(),
        )?;

        let attestor_id = attestation.attestor_id();
        if !is_valid_attestor {
            info!(target: LOG_TARGET, "📝 Attestor is not valid for attestation epoch: {}", current_epoch);
            return Err(Error::NotAnAttestor(attestor_id));
        }

        // Get randomness from the attestation
        let attestation_epoch = attestation.proof_of_inclusion.epoch;
        let runtime = self.runtime.runtime_api();
        let randomness = runtime.randomness_by_epoch_id(best_hash, attestation_epoch)?;

        // Here we verify the proof of inclusion
        // based on the round config
        // Get round config at the attestation epoch
        let round_config =
            round::get_round_config(self.runtime.clone(), chain_key, best_hash, &self.state)?;
        let is_included = vrf::verify_proof_of_inclusion(
            round_config.committee_set_size.into(),
            round_config.target_sample_size.into(),
            &randomness,
            &attestation.proof_of_inclusion,
            &attestor_id,
            header_number,
        )?;

        if !is_included {
            warn!(target: LOG_TARGET, "📝 Attestor {:?} not eligible", attestor_id);
            return Err(Error::AttestorNotEligible(attestor_id));
        }

        debug!(target: LOG_TARGET, "📝 Attestor {:?} selected ✅", attestor_id);
        Ok(())
    }

    /// In practice, this method would:
    /// 1. Gather all attesations for a round, create a BLS signature
    /// 2. Submit the inherent transaction containing the attestation
    /// 3. Flush memory
    fn try_submit_attestation(
        &mut self,
        round: Round,
        attestation: Attestation<HashFor<B>, AccountId>,
    ) -> Result<(), Error> {
        let chain_key = round.0;
        let header_number = round.1;

        let attestations = self
            .state
            .get_attestations_by_chain_and_header(chain_key, header_number)?;
        // here goes bls
        // contains attestorid, and attestation itself.
        let mut attestors = Vec::with_capacity(attestations.len());

        // Using iter() or into_iter() based on whether raw_attestations is needed later
        let (attestors_collected, signatures): (Vec<_>, Vec<<Bls as CryptoScheme>::Signature>) =
            attestations
                .iter() // or into_iter() if we can consume raw_attestations
                .map(|(attestor_bls_pubkey, attestations)| {
                    (
                        attestor_bls_pubkey.clone(),
                        attestations.signature_bls.clone(),
                    ) // Clone if necessary
                })
                .unzip();

        attestors.extend(attestors_collected);

        // retrieve inner bls signature
        let sigs = signatures
            .iter()
            .map(|WrapEncode(sig)| *sig)
            .collect::<Vec<_>>();

        let aggregated_signature = aggregate(&sigs[..])
            .ok()
            .and_then(|sig| sig.as_bytes().try_into().ok())
            .ok_or(Error::InvalidBlsSignature)?;

        let attestation = SignedAttestation {
            attestation: attestation.attestation_data,
            signature: aggregated_signature,
            attestors,
        };

        let _ = match self.inherent_provider.0.lock() {
            Ok(mut provider) => match provider.create(attestation) {
                Ok(()) => {
                    debug!(target: LOG_TARGET, "📝 Inherent created");
                    Ok(())
                }
                Err(e) => {
                    error!(target: LOG_TARGET, "📝 Error creating inherent: {:?}", e);
                    Err(Error::ErrorCreatingInherent)
                }
            },
            Err(e) => {
                error!("error acquiring lock, {:?}", e);
                Ok(())
            }
        };

        // Flush memory
        self.state.clear_votes(chain_key, header_number);

        Ok(())
    }

    /// Handle finality notification
    /// This function updates all the round configurations for each supported chain when a new epoch is finalized
    fn handle_finality_notification(&mut self, notif: &FinalityNotification<B>) -> Result<(), Error>
    where
        B: BlockT,
    {
        info!(target: LOG_TARGET, "📝 Handling finality notification");
        let runtime_api = self.runtime.runtime_api();

        // get current epoch
        let current_epoch = runtime_api.current_epoch(notif.hash)?;

        if current_epoch.epoch_index == 0 {
            info!(target: LOG_TARGET, "📝 Skipping round config for epoch 0");
            return Ok(());
        }

        if self.current_epoch_index == current_epoch.epoch_index && current_epoch.epoch_index != 0 {
            debug!(target: LOG_TARGET, "📝 No need to update round configuration for current epoch");
            return Ok(());
        }

        info!(target: LOG_TARGET, "📝 Updating round configuration for epoch: {:?}", current_epoch.epoch_index);

        // Get supported chain keys
        let supported_chain_keys = runtime_api.supported_chains(notif.hash)?;

        // Update round config for each supported chain
        for chain_key in supported_chain_keys {
            // Get active attestor set
            let active_attestors = runtime_api.active_attestor_set(notif.hash, chain_key)?;

            self.state.update_active_attestors(
                chain_key,
                current_epoch.epoch_index,
                active_attestors,
            );
        }

        // Update current epoch index
        self.current_epoch_index = current_epoch.epoch_index;

        Ok(())
    }
}
