use crate::metrics::register_metrics;
use crate::round::RoundConfig;
use crate::{metric_inc, metric_inc_chain, metric_set_chain, metrics::VoterMetrics};
use attestor_primitives::ChainKey;
use futures::{stream::Fuse, StreamExt};
use log::{debug, error, info, warn};
use parity_scale_codec::{Codec, Decode, Encode};
use sc_client_api::{Backend, BlockBackend, HeaderBackend};
use sc_network::NetworkPeers;
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
    SignedAttestation,
};

use bls_signatures::{aggregate, Serialize};
use randomness_primitives::api::RandomnessPalletApi;
use supported_chains_primitives::api::SupportedChainsApi;

use super::{inherent, AttestorComms, Client, HashFor, Message, LOG_TARGET};
use crate::communication::{validator::GossipFilterCfg, Attestation, Error};
use crate::state::{State, VoteImportResult};
use crate::{round, UnpinnedFinalityNotification};

/// Gossip engine votes messages topic
pub(crate) fn votes_topic<B: BlockT>() -> B::Hash {
    <<B::Header as HeaderT>::Hashing as HashT>::hash(b"attestor-votes")
}

pub(crate) struct Worker<B: BlockT, RuntimeApi: ProvideRuntimeApi<B>, BE, C, AccountId, S, N>
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
    pub comms: AttestorComms<B, AccountId, N>,

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
}

pub(crate) struct WorkerParams<B: BlockT, RuntimeApi: ProvideRuntimeApi<B>, BE, C, AccountId, S, N>
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
    pub comms: AttestorComms<B, AccountId, N>,

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

impl<B: BlockT, RA: ProvideRuntimeApi<B>, BE, C, AccountId, S, N>
    Worker<B, RA, BE, C, AccountId, S, N>
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
    N: NetworkPeers,
{
    pub fn new(params: WorkerParams<B, RA, BE, C, AccountId, S, N>) -> Self {
        let metrics = register_metrics(params.prometheus_registry);
        Worker {
            comms: params.comms,
            runtime: params.runtime.clone(),
            client: params.client,
            state: State::default(),
            current_epoch_index: 0,
            backend: params.backend.clone(),
            inherent_provider: params.inherent_provider,
            metrics,
            is_authority: params.is_authority,
            sync: params.sync,
        }
    }

    pub async fn start(
        mut self,
        finality_notifications: &mut Fuse<crate::FinalityNotifications<B>>,
    ) -> (Error, AttestorComms<B, AccountId, N>) {
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
                        debug!(target: LOG_TARGET, "📝 Finality notification processed");
                    } else {
                        break Error::FinalityStreamTerminated;
                    }
                },
                // Make sure to pump gossip engine.
                _ = gossip_engine => {
                    break Error::GossipEngineExited;
                },
                // Handler that handles incoming attestation from the gossip netowrk
                attestation = votes.next() => {
                    if let Some(attestation) = attestation {
                        if self.triage_message(attestation, false).is_ok() {
                            debug!(target: LOG_TARGET, "📝 Attestation from gossip network processed");
                        } else {
                            warn!(target: LOG_TARGET, "📝 Attestation from gossip network failed to process");
                            metric_inc!(self.metrics, attestor_invalid_votes);
                        }
                    } else {
                        break Error::GossipEngineExited;
                    }
                },
                // Handler that handles incoming attestation from it's rpc endpoint
                // This is the main entry point for the attestation worker
                // It will handle incoming attestations, and gossip them to the network
                message = message_stream.next() => {
                    if let Some(message) = message {
                        if self.triage_message(message, true).is_ok() {
                            debug!(target: LOG_TARGET, "📝 Attestation from rpc endpoint processed");
                        } else {
                            warn!(target: LOG_TARGET, "📝 Attestation from rpc endpoint failed to process");
                            metric_inc!(self.metrics, attestor_invalid_votes);
                        }
                    }
                }
            }
        };

        (error, self.comms)
    }

    fn triage_message(
        &mut self,
        message: Message<B, AccountId>,
        from_rpc: bool,
    ) -> Result<(), Error> {
        match message {
            Message::Attestation(attestation) => {
                let chain_key = attestation.chain_key();
                let digest = attestation.digest();
                let round = attestation.round();

                if from_rpc {
                    debug!(
                        target: LOG_TARGET,
                        "📝 RPC: Got an attestation for round: {:?}, with digest {:?}, from attestor {}",
                        round,
                        digest,
                        attestation.attestor_id().account_id()
                    );
                    metric_inc_chain!(self.metrics, attestor_votes_from_rpc_per_chain, chain_key);
                } else {
                    debug!(
                        target: LOG_TARGET,
                        "📝 GOSSIP: Got an attestation for round: {:?}, with digest {:?}, from attestor {}",
                        round,
                        digest,
                        attestation.attestor_id().account_id()
                    );
                    metric_inc_chain!(self.metrics, attestor_imported_votes_per_chain, chain_key);
                }

                match self.process_attestation_message(attestation, from_rpc) {
                    Ok(()) => {
                        metric_inc_chain!(
                            self.metrics,
                            attestor_good_votes_processed_per_chain,
                            chain_key
                        );
                        debug!(target: LOG_TARGET, "📝 Attestation processed for round: {round:?}, with digest {digest:?}");
                    }
                    Err(e) => {
                        debug!(target: LOG_TARGET, "📝 Error for attestation for round: {round:?}, with digest {digest:?}, err: {e:?}");
                    }
                }

                Ok(())
            }
        }
    }

    /// Process attestation message
    /// Takes care of importing the vote, validating it, gossiping it if from rpc
    /// and finalizing it if the round is concluded (submitting an inherent)
    fn process_attestation_message(
        &mut self,
        attestation: Attestation<HashFor<B>, AccountId>,
        from_rpc: bool,
    ) -> Result<(), Error> {
        let chain_key = attestation.chain_key();

        if self.sync.is_major_syncing() {
            warn!(target: LOG_TARGET, "📝 Node is syncing, skipping message for digest {:?}", attestation.digest());
            return Err(Error::WorkerInSync);
        }
        metric_set_chain!(
            self.metrics,
            attestor_best_block_per_chain,
            chain_key,
            attestation.header_number()
        );

        // First we Validate the vote
        let finalized_block_hash = self.backend.blockchain().info().finalized_hash;
        self.validate_attestation(finalized_block_hash, &attestation)?;

        // Then we import the vote.
        let import_result = self.state.note_vote(attestation.clone(), from_rpc)?;

        // in cases where the vote is already imported or stale we don't validate it further because we don't have to
        let round = attestation.round();
        match import_result {
            VoteImportResult::DoubleVote => {
                warn!(target: LOG_TARGET, "📝 Double vote detected, round: {:?} for digest {:?}", round, attestation.digest());
                metric_inc_chain!(
                    self.metrics,
                    attestor_equivocation_votes_per_chain,
                    chain_key
                );
                return Err(Error::DoubleVote);
            }
            VoteImportResult::Ok => {
                metric_set_chain!(
                    self.metrics,
                    attestor_best_voted_per_chain,
                    chain_key,
                    attestation.header_number()
                );
                info!(target: LOG_TARGET, "📝 Attestation added to round: {:?} for digest {:?}", round, attestation.digest());
            }
            VoteImportResult::Stale => {
                info!(target: LOG_TARGET, "📝 Stale vote detected, round: {:?} for digest {:?}", round, attestation.digest());
                metric_inc_chain!(self.metrics, attestor_stale_votes_per_chain, chain_key);
                return Err(Error::StaleVote);
            }
            VoteImportResult::RoundConcluded => {
                info!(target: LOG_TARGET, "📝 Round {:?} concluded for digest {:?}", round, attestation.digest());
            }
        }

        // Gossip now
        metric_inc_chain!(self.metrics, attestor_votes_sent_per_chain, chain_key);
        debug!(target: LOG_TARGET,
            "📝 Will gossip attestation with digest {:?}, for chain_key {:?}, from attestor {}",
            attestation.digest(),
            chain_key,
            attestation.attestor_id().account_id()
        );
        self.comms.gossip_engine.gossip_message(
            votes_topic::<B>(),
            Message::<B, AccountId>::Attestation(attestation.clone()).encode(),
            false,
        );

        // If we are concluded, finalize the vote
        if self.state.is_concluded(&round) {
            self.finalize_vote(attestation)?;
        }

        Ok(())
    }

    // Finalize the vote
    // This function is responsible for finalizing the vote
    // We can only finalize if we are an authority
    // It will also update the gossip filter based on the finalized block hash, chain key and header number
    fn finalize_vote(
        &mut self,
        attestation: Attestation<HashFor<B>, AccountId>,
    ) -> Result<(), Error> {
        let round = attestation.round();
        let chain_key = attestation.chain_key();
        let header_number = attestation.header_number();

        // Submit attestation
        // Only submit if we are an authority
        if self.is_authority {
            match self.try_submit_attestation(attestation) {
                Ok(()) => {
                    debug!(target: LOG_TARGET, "📝 Attestation for round: {round:?} submitted");
                }
                Err(e) => {
                    error!(target: LOG_TARGET, "📝 Error submitting attestation: {e:?}");
                }
            }
        }

        // Flush memory
        self.state.clear_votes(round.0, round.1);

        // Calculate new start
        let finalized_block_hash = self.backend.blockchain().info().finalized_hash;
        let runtime_api = self.runtime.runtime_api();
        let attestation_interval =
            runtime_api.chain_attestation_interval(finalized_block_hash, chain_key)?;
        let new_start = header_number + attestation_interval;

        // Update the gossip filter
        // We will not accept any more votes for this round since it is finalized
        self.update_gossip_filter(
            finalized_block_hash,
            chain_key,
            new_start,
            self.current_epoch_index,
        )?;

        Ok(())
    }

    /// In practice, this method would:
    /// 1. Gather all attesations for a round, create a BLS signature
    /// 2. Submit the inherent transaction containing the attestation
    /// 3. Flush memory
    fn try_submit_attestation(
        &mut self,
        attestation: Attestation<HashFor<B>, AccountId>,
    ) -> Result<(), Error> {
        let digest_to_conclude = attestation.digest();

        let chain_key = attestation.chain_key();
        let header_number = attestation.header_number();

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
                .filter_map(|(attestor_bls_pubkey, attestation)| {
                    if attestation.digest() == digest_to_conclude {
                        Some((
                            attestor_bls_pubkey.clone(),
                            attestation.signature_bls.clone(),
                        )) // Clone if necessary
                    } else {
                        None
                    }
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

        match self.inherent_provider.0.lock() {
            Ok(mut provider) => match provider.create(attestation) {
                Ok(()) => {
                    debug!(target: LOG_TARGET, "📝 Inherent created");
                    Ok(())
                }
                Err(e) => {
                    error!(target: LOG_TARGET, "📝 Error creating inherent: {e:?}");
                    Err(Error::ErrorCreatingInherent)
                }
            },
            Err(e) => {
                error!("error acquiring lock, {e:?}");
                Ok(())
            }
        }?;

        Ok(())
    }

    // Get or create round config
    // This function retrieves or creates a round configuration based on the provided chain key.
    // Safeguards creation if not exists when the worker is restarted.
    pub fn get_or_create_round_config(
        &mut self,
        at: B::Hash,
        chain_key: ChainKey,
    ) -> Result<RoundConfig, Error> {
        let round_config = self.state.get_round_config(chain_key);

        if let Some(round_config) = round_config {
            return Ok(round_config.clone());
        }

        // Round config is reset, create one
        let runtime_api = self.runtime.runtime_api();
        let current_epoch = runtime_api.current_epoch(at)?;

        let round_config = round::create(
            self.runtime.clone(),
            chain_key,
            at,
            current_epoch.epoch_index,
        )?;
        // Update round configuration
        self.state.add_round_config(chain_key, round_config.clone());

        Ok(round_config)
    }

    /// Handle finality notification
    /// This handler is repsonsible for updating gossip messager filter
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

        info!(target: LOG_TARGET, "📝 Handling finality notification");

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

        // Get supported chain keys
        let supported_chain_keys = runtime_api.supported_chains(notif.hash)?;

        // Update gossip filter for each supported chain
        for chain_key in supported_chain_keys {
            debug!(target: LOG_TARGET, "📝 Updating round configuration for epoch: {:?}", current_epoch.epoch_index);
            let round_config = round::create(
                self.runtime.clone(),
                chain_key,
                notif.hash,
                current_epoch.epoch_index,
            )?;
            // Update round configuration
            self.state.add_round_config(chain_key, round_config.clone());

            // Returns the best known header number for `chain_key`:
            // 1) last attested header (if present)
            // 2) last checkpoint
            // 3) chain genesis block number
            let last_header_number = {
                // Common fallback path (checkpoint → genesis)
                let fallback = || -> Result<_, _> {
                    if let Some(checkpoint) = runtime_api.last_checkpoint(notif.hash, chain_key)? {
                        debug!(target: LOG_TARGET, "📝 Using last checkpoint for chain key: {chain_key:?}");
                        Ok(checkpoint.block_number)
                    } else {
                        debug!(target: LOG_TARGET, "📝 Allowing bootstrap of chain: {chain_key}");
                        runtime_api.attestation_chain_genesis_block_number(notif.hash, chain_key)
                    }
                };

                let last_attested_digest = runtime_api.last_digest(notif.hash, chain_key)?;
                if let Some(digest) = last_attested_digest {
                    match runtime_api.get(notif.hash, chain_key, digest)? {
                        Some(last_attested_header) => {
                            debug!(target: LOG_TARGET, "📝 Using last attested header for chain key: {chain_key:?}");
                            Ok(last_attested_header.header_number())
                        }
                        None => fallback(),
                    }
                } else {
                    fallback()
                }?
            };

            self.update_gossip_filter(
                notif.hash,
                chain_key,
                last_header_number,
                current_epoch.epoch_index,
            )?;
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
        epoch: u64,
    ) -> Result<(), Error> {
        if epoch == 0 {
            debug!(target: LOG_TARGET, "📝 Skipping gossip filter update for epoch 0");
            return Ok(());
        }

        let runtime_api = self.runtime.runtime_api();
        // Get active attestor set
        let active_attestors = runtime_api.active_attestor_set(block_hash, chain_key)?;

        if active_attestors.is_empty() {
            debug!(target: LOG_TARGET, "📝 Not setting filter for chain: {chain_key:?} because there are no attestors");
            return Ok(());
        }

        // Get attestation interval, checkpoint interval and vote acceptance window
        let attestation_interval = runtime_api.chain_attestation_interval(block_hash, chain_key)?;
        let checkpoint_interval =
            runtime_api.attestation_checkpoint_interval(block_hash, chain_key)?;
        let vote_acceptance_window =
            runtime_api.chain_vote_acceptance_window(block_hash, chain_key)?;

        let window_in_blocks =
            attestation_interval * (checkpoint_interval as u64) * vote_acceptance_window;

        // Have a sliding window of 10 checkpoints where attestations are valid
        debug!(target: LOG_TARGET, "📝 Updating gossip filter for chain key: {}, epoch: {}, start: {}, window_in_blocks: {}, active_attestors: {}",
            chain_key,
            epoch,
            current_block,
            window_in_blocks,
            active_attestors.len()
        );
        self.comms.gossip_validator.update_filter(GossipFilterCfg {
            chain_key,
            epoch,
            start: current_block,
            window: window_in_blocks,
            attestors: &active_attestors,
        });
        Ok(())
    }
}
