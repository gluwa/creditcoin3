use crate::metrics::register_metrics;
use crate::{metric_inc, metric_set, metrics::VoterMetrics};
use attestor_primitives::ChainKey;
use futures::{stream::Fuse, StreamExt};
use log::{debug, error, info, warn};
use parity_scale_codec::{Codec, Decode, Encode};
use sc_client_api::{Backend, BlockBackend, HeaderBackend};
use sp_api::ProvideRuntimeApi;
use sp_consensus::SyncOracle;
use sp_consensus_babe::BabeApi;
use sp_core::H256;
use sp_runtime::traits::{Block as BlockT, Hash as HashT, Header as HeaderT};
use std::fmt::{Debug, Display};
use std::sync::Arc;
use substrate_prometheus_endpoint::Registry;

use attestor_primitives::{
    api::AttestorApi,
    bls::{Bls, CryptoScheme, WrapEncode},
    Round, SignedAttestation,
};

use bls_signatures::{aggregate, Serialize};
use randomness_primitives::api::RandomnessPalletApi;
use supported_chains_primitives::api::SupportedChainsApi;

use super::{inherent, AttestorComms, Client, HashFor, Message, LOG_TARGET};
use crate::communication::{Attestation, Error};
use crate::metrics::VoterMetrics;
use crate::state::{State, VoteImportResult};
use crate::validate::AttestationValidator;
use crate::{round, UnpinnedFinalityNotification};

/// Gossip engine votes messages topic
pub(crate) fn votes_topic<B: BlockT>() -> B::Hash {
    <<B::Header as HeaderT>::Hashing as HashT>::hash(b"attestor-votes")
}

pub(crate) struct Worker<B: BlockT, RuntimeApi: ProvideRuntimeApi<B>, BE, C, AccountId, S>
where
    RuntimeApi: ProvideRuntimeApi<B> + Send + Sync + 'static,
    RuntimeApi::Api: BabeApi<B>,
    RuntimeApi::Api: AttestorApi<B, HashFor<B>, AccountId>,
    RuntimeApi::Api: SupportedChainsApi<B>,
    RuntimeApi::Api: RandomnessPalletApi<B>,
    BE: Backend<B> + 'static,
    S: SyncOracle,
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
    pub comms: AttestorComms<B, AccountId>,

    /// runtime api access
    pub runtime: Arc<RuntimeApi>,

    #[allow(dead_code)]
    pub client: Arc<C>,

    /// Client Backend
    pub backend: Arc<BE>,

    pub state: State<HashFor<B>, AccountId>,

    /// Current epoch index
    pub current_epoch_index: u64,

    pub inherent_provider: inherent::AsyncProvider<AccountId, B, RuntimeApi, BE>,

    pub metrics: Option<VoterMetrics>,

    /// If the worker is an authority
    is_authority: bool,

    sync: Arc<S>,

    /// Validator for attestations
    pub attestation_validator: AttestationValidator<B, AccountId, RuntimeApi>,
}

pub(crate) struct WorkerParams<B: BlockT, RuntimeApi: ProvideRuntimeApi<B>, BE, C, AccountId, S>
where
    RuntimeApi: ProvideRuntimeApi<B> + Send + Sync + 'static,
    RuntimeApi::Api: BabeApi<B>,
    RuntimeApi::Api: AttestorApi<B, HashFor<B>, AccountId>,
    RuntimeApi::Api: SupportedChainsApi<B>,
    RuntimeApi::Api: RandomnessPalletApi<B>,
    BE: Backend<B> + 'static,
    S: SyncOracle,
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
    pub comms: AttestorComms<B, AccountId>,

    /// runtime api access
    pub runtime: Arc<RuntimeApi>,

    pub client: Arc<C>,

    /// Client Backend
    pub backend: Arc<BE>,

    pub inherent_provider: inherent::AsyncProvider<AccountId, B, RuntimeApi, BE>,

    /// Gossip sync
    pub sync: Arc<S>,

    /// If the worker is an authority
    pub is_authority: bool,

    pub prometheus_registry: Option<Registry>,
}

impl<B: BlockT, RA: ProvideRuntimeApi<B>, BE, C, AccountId, S> Worker<B, RA, BE, C, AccountId, S>
where
    B: BlockT,
    RA: ProvideRuntimeApi<B> + Send + Sync + 'static,
    RA::Api: BabeApi<B>,
    RA::Api: AttestorApi<B, HashFor<B>, AccountId>,
    RA::Api: SupportedChainsApi<B>,
    RA::Api: RandomnessPalletApi<B>,
    BE: Backend<B>,
    C: Client<B, BE> + BlockBackend<B>,
    H256: From<<B as BlockT>::Hash>,
    <B as BlockT>::Hash: From<H256>,
    <<B as BlockT>::Header as HeaderT>::Number: Into<u64>,
    S: SyncOracle,
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
    pub fn new(params: WorkerParams<B, RA, BE, C, AccountId, S>) -> Self {
        let block_hash = params.backend.blockchain().info().finalized_hash;

        let current_epoch_index = params
            .runtime
            .runtime_api()
            .current_epoch(block_hash)
            .expect("Failed to get current epoch index");

        let metrics = register_metrics(params.prometheus_registry);
        Worker {
            comms: params.comms,
            runtime: params.runtime.clone(),
            client: params.client,
            state: State::default(),
            current_epoch_index: current_epoch_index.epoch_index,
            backend: params.backend.clone(),
            inherent_provider: params.inherent_provider,
            metrics,
            is_authority: params.is_authority,
            sync: params.sync,
            attestation_validator: AttestationValidator::new(params.runtime.clone()),
        }
    }

    pub async fn start(
        mut self,
        finality_notifications: &mut Fuse<crate::FinalityNotifications<B>>,
    ) -> (Error, AttestorComms<B, AccountId>) {
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
        let error = loop {
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
                        metric_inc!(self.metrics, attestor_imported_votes);
                        match self.triage_message(vote.clone()).await {
                            Ok(()) => {
                                metric_inc!(self.metrics, attestor_good_votes_processed);
                                debug!(target: LOG_TARGET, "📝 Got a valid gossiped message");
                            },
                            Err(e) => {
                                debug!(target: LOG_TARGET, "📝 Got error for message err: {:?}", e);
                            }
                        }
                    } else {
                        warn!(target: LOG_TARGET, "📝 Got a vote, but it was invalid");
                        metric_inc!(self.metrics, attestor_invalid_votes);
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
                                metric_inc!(self.metrics, attestor_votes_from_rpc);
                                let chain_key = attestation.attestation_data.chain_key;
                                let header_number = attestation.attestation_data.header_number;

                                let round = (chain_key, header_number);
                                debug!(target: LOG_TARGET, "📝 Got attestation to gossip with digest {:?}, on topic: {:?} for round {:?}", attestation.digest(), topic, round);

                                metric_inc!(self.metrics, attestor_good_votes_processed);
                                // Also process the message
                                match self.process_attestation_message(attestation).await {
                                    Ok(()) => {
                                        debug!(target: LOG_TARGET, "📝 Got a valid incoming message from rpc, round: {:?}", round);
                                        // Gossip now
                                        metric_inc!(self.metrics, attestor_votes_sent);
                                        self.comms.gossip_engine.gossip_message(
                                            topic,
                                            message.encode(),
                                            false,
                                        );
                                    },
                                    Err(e) => {
                                        debug!(target: LOG_TARGET, "📝 Got error for message err: {:?}", e);
                                    }
                                }

                            },
                        };
                    }
                }
            }
        };

        (error, self.comms)
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
        if self.sync.is_major_syncing() {
            warn!(target: LOG_TARGET, "📝 Node is syncing, skipping message");
            return Err(Error::WorkerInSync);
        }

        let block_hash = self.backend.blockchain().info().best_hash;

        // Get the round for the attestation
        // This is the chain key and header number
        let round = attestation.round();

        // Validate the attestation
        self.attestation_validator
            .validate_attestation(block_hash, round, &attestation)?;

        // Verify the VRF output
        self.verify_vrf(block_hash, round, &attestation)?;

        // Short circuit if we are not an authority
        if !self.is_authority {
            metric_inc!(self.metrics, attestor_no_authority_found_in_store);
            debug!(target: LOG_TARGET, "📝 Not an authority, skipping counting votes");
            return Ok(());
        }

        let round_config = round::get_round_config(self.runtime.clone(), round.0, block_hash)?;

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
                metric_inc!(self.metrics, attestor_equivocation_votes);
            }
            VoteImportResult::Ok => {
                let block_number = self.backend.blockchain().info().best_number;
                let block_number: u64 = block_number.into();
                metric_set!(self.metrics, attestor_best_voted, block_number);
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
        best_hash: B::Hash,
        round: Round,
        attestation: &Attestation<HashFor<B>, AccountId>,
    ) -> Result<(), Error> {
        debug!(target: LOG_TARGET, "📝 Verifying VRF output for attestation, round: {:?}", round);
        let chain_key = round.0;
        let header_number = round.1;

        let attestor_id = attestation.attestor_id();

        // Get randomness from the attestation
        let attestation_epoch = attestation.proof_of_inclusion.epoch;
        let runtime = self.runtime.runtime_api();
        let randomness = runtime.randomness_by_epoch_id(best_hash, attestation_epoch)?;

        // Here we verify the proof of inclusion
        // based on the round config
        // Get round config at the attestation epoch
        let round_config = round::get_round_config(self.runtime.clone(), chain_key, best_hash)?;
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
    /// This handlers is repsonsible for updating gossip messager filter
    /// It will check if the current epoch has changed, and update the gossip filter accordingly
    /// This is done by getting the active attestor set, and the last finalized attestation
    /// Based on that we allow attestations to be gossip for a sliding window of 2 checkpoints
    /// This means we can allow nodes to gossip attestations for 2 checkpoints before and after the last finalized attestation
    fn handle_finality_notification(
        &mut self,
        notif: &UnpinnedFinalityNotification<B>,
    ) -> Result<(), Error>
    where
        B: BlockT,
    {
        if self.sync.is_major_syncing() {
            warn!(target: LOG_TARGET, "📝 Node is syncing, skipping finality notifications");
            return Ok(());
        }

        let header = &notif.header;
        debug!(
            target: LOG_TARGET,
            "📝 Finality notification: header(number {:?}, hash {:?}) tree_route {:?}",
            header.number(),
            notif.header.hash(),
            notif.tree_route,
        );

        let runtime_api = self.runtime.runtime_api();

        // get current epoch
        let current_epoch = runtime_api.current_epoch(notif.hash)?;
        if current_epoch.epoch_index == 0 {
            debug!(target: LOG_TARGET, "📝 Skipping round config for epoch 0");
            return Ok(());
        }

        // If the epoch is the same and current epoch we don't need to update filter
        if self.current_epoch_index == current_epoch.epoch_index {
            debug!(target: LOG_TARGET, "📝 No need to update round configuration for current epoch");
            return Ok(());
        }

        debug!(target: LOG_TARGET, "📝 Updating round configuration for epoch: {:?}", current_epoch.epoch_index);

        // Get supported chain keys
        let supported_chain_keys = runtime_api.supported_chains(notif.hash)?;

        // Update gossip filter for each supported chain
        for chain_key in supported_chain_keys {
            let last_header_number = if let Some(digest) =
                runtime_api.last_digest(notif.hash, chain_key)?
            {
                if let Some(attestation) = runtime_api.get(notif.hash, chain_key, digest)? {
                    attestation.header_number()
                } else {
                    debug!(target: LOG_TARGET, "📝 No last attestation found for chain key: {:?}", chain_key);
                    return Err(Error::GossipEngineExited);
                }
            } else {
                debug!(target: LOG_TARGET, "📝 Allowing bootstrap of chain: {chain_key}");
                0
            };

            self.update_gossip_filter(notif.hash, chain_key, last_header_number)?;
        }

        // Update current epoch index
        self.current_epoch_index = current_epoch.epoch_index;

        Ok(())
    }

    fn update_gossip_filter(
        &self,
        block_hash: B::Hash,
        chain_key: ChainKey,
        current_block: u64,
    ) -> Result<(), Error> {
        let runtime_api = self.runtime.runtime_api();
        // Get active attestor set
        let active_attestors = runtime_api.active_attestor_set(block_hash, chain_key)?;

        if active_attestors.is_empty() {
            debug!(target: LOG_TARGET, "📝 Not setting filter for chain: {:?} because there are no attestors", chain_key);
            return Ok(());
        }

        // Get attestation interval and checkpoint interval
        let attestation_interval = runtime_api.chain_attestation_interval(block_hash, chain_key)?;
        let checkpoint_interval =
            runtime_api.attestation_checkpoint_interval(block_hash, chain_key)?;

        // Calculate one checkpoints in number of attestations
        let two_checkpoints = attestation_interval * checkpoint_interval as u64 * 2;

        // Have a sliding window of 2 checkpoints where attestations are valid
        info!(target: LOG_TARGET, "📝 Updating gossip filter for chain key: {:?}", chain_key);
        self.comms.gossip_validator.update_filter(
            chain_key,
            current_block.saturating_sub(two_checkpoints),
            current_block
                .checked_add(two_checkpoints)
                .ok_or(Error::Overflow)?,
            active_attestors,
        );
        metric_set!(self.metrics, attestor_best_block, current_block);
        Ok(())
    }
}
