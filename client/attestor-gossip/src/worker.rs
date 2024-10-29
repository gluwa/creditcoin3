use futures::StreamExt;
use log::{debug, error, info};
use parity_scale_codec::{Codec, Decode, Encode};
use sc_client_api::{Backend, BlockBackend, HeaderBackend};
use sp_api::ProvideRuntimeApi;
use sp_consensus_babe::BabeApi;
use sp_core::H256;
use sp_inherents::CreateInherentDataProviders;
use sp_runtime::traits::{Block as BlockT, Hash as HashT, Header as HeaderT};
use std::collections::{BTreeMap, HashMap};
use std::fmt::{Debug, Display};
use std::marker::PhantomData;
use std::sync::Arc;

use attestor_primitives::{
    api::AttestorApi,
    bls::{Bls, CryptoScheme, WrapEncode},
    AttestorId, AttestorStatus, ChainKey, SignedAttestation,
};

use bls_signatures::{aggregate, Serialize};
use randomness_primitives::api::RandomnessPalletApi;
use supported_chains_primitives::api::SupportedChainsApi;
use vrf;

use super::{inherent, Attestation, AttestorComms, Client, Error, HashFor, Message, LOG_TARGET};

/// Gossip engine votes messages topic
pub(crate) fn votes_topic<B: BlockT>() -> B::Hash
where
    B: BlockT,
{
    <<B::Header as HeaderT>::Hashing as HashT>::hash(b"attestor-votes")
}

type BlockNumber = u64;

type Attestations<B, AccountId> = HashMap<AccountId, Attestation<HashFor<B>, AccountId>>;

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

    /// Block attestations. Maps a blocknumber to a list of actual attestations, not digests
    pub block_attestations: BTreeMap<ChainKey, BTreeMap<BlockNumber, Attestations<B, AccountId>>>,

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
        Worker {
            comms: params.comms,
            runtime: params.runtime,
            client: params.client,
            create_inherent_data_providers: params.create_inherent_data_providers,
            block_attestations: BTreeMap::new(),
            backend: params.backend,
            inherent_provider: params.inherent_provider,
            _phantom: PhantomData,
        }
    }

    pub async fn start(mut self) -> Error {
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
                // Make sure to pump gossip engine.
                _ = gossip_engine => {
                    break Error::GossipEngineExited;
                },
                // Handler that handles incoming attestation from the gossip netowrk
                vote = votes.next() => {
                    if let Some(vote) = vote {
                        log::info!(target: LOG_TARGET, "📝 Got a vote from the network");
                        match self.triage_message(vote.clone()).await {
                            Ok(()) => {
                                info!(target: LOG_TARGET, "📝 Got a valid gossiped message");
                            },
                            Err(e) => {
                                info!(target: LOG_TARGET, "📝 Got error for message err: {:?}", e);
                            }
                        }
                    } else {
                        info!(target: LOG_TARGET, "📝 Got a vote, but it was invalid");
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
                                info!(target: LOG_TARGET, "📝 Got attestation to gossip with digest {:?}, on topic: {:?} for round {:?}", attestation.digest(), topic, round);

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
                            }
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

        // Verify the attestation data
        self.verify_attestation_data(&attestation)?;

        // Verify the VRF output
        self.verify_vrf(&attestation)?;

        let chain_key = attestation.attestation_data.chain_key;
        let header_number = attestation.attestation_data.header_number;

        self.add_to_round(attestation, block_hash)?;

        match self.try_submit_attestation(chain_key, header_number, block_hash) {
            Ok(()) => {
                info!(target: LOG_TARGET, "📝 Successfully submitted attestation");
            }
            Err(e) => {
                info!(target: LOG_TARGET, "📝 Failed to submit attestation err: {:?}", e);
            }
        }

        Ok(())
    }

    /// Verify the VRF output for an attestation.
    /// This checks if the attestor that submitted this attestations vrf output is correct
    /// Correct being, that it signed the babe's VRF output from Two epochs ago & that the attestor is eligible to submit an attestation
    /// TODO: check if the eligibility check is good enough
    fn verify_vrf(&self, attestation: &Attestation<HashFor<B>, AccountId>) -> Result<(), Error> {
        // Get the blockchain info (current info)
        let blockchain_info = self.backend.blockchain().info();
        let chain_key = attestation.attestation_data.chain_key;

        info!(target: LOG_TARGET, "📝 Verifying VRF output for attestation");
        let runtime = self.runtime.runtime_api();
        let is_attestor = runtime.is_attestor(
            blockchain_info.finalized_hash,
            chain_key,
            &attestation.attestor.clone(),
        )?;
        debug!(target: LOG_TARGET, "📝 {} Is attestor {}", attestation.attestor, is_attestor);

        if !is_attestor {
            return Err(Error::NotAnAttestor);
        }

        let attestor_status = runtime
            .attestor_status(
                blockchain_info.finalized_hash,
                chain_key,
                &attestation.attestor.clone(),
            )?
            .ok_or(Error::NotAnAttestor)?;

        if attestor_status != AttestorStatus::Active {
            return Err(Error::AttestorNotActive);
        }

        let last_digest = runtime.last_digest(
            blockchain_info.finalized_hash,
            attestation.attestation_data.chain_key,
        )?;

        let prev_digest = attestation.attestation_data.prev_digest;

        if last_digest != prev_digest {
            info!(target: LOG_TARGET, "📝 last digest: {:?}, attestation digest {:?}", last_digest, prev_digest);

            return Err(Error::DigestMissMatch);
        }

        // Get randomness from the attestation
        info!(target: LOG_TARGET, "Getting randomness for attestation at epoch: {}", attestation.proof_of_inclusion.epoch);
        let runtime = self.runtime.runtime_api();
        let randomness_from_attestation = runtime.randomness_by_epoch_id(
            blockchain_info.finalized_hash,
            attestation.proof_of_inclusion.epoch,
        )?;

        let current_epoch = runtime.current_epoch(blockchain_info.finalized_hash)?;
        let two_epochs_ago = current_epoch.epoch_index.saturating_sub(2);

        let randomness_from_two_epochs_ago =
            runtime.randomness_by_epoch_id(blockchain_info.finalized_hash, two_epochs_ago)?;

        // Enforce that an attestor can only submit an attestation if they signed the VRF output from two epochs ago
        // calculated from "now" which means the current epoch on a synced node
        if randomness_from_attestation != randomness_from_two_epochs_ago {
            info!(target: LOG_TARGET, "📝 Randomness from attestation: {:?}, randomness from two epochs ago: {:?}", randomness_from_attestation, randomness_from_two_epochs_ago);
            return Err(Error::InvalidAttestationVrfOuput);
        }

        // Get the threshold and working set size
        let runtime = self.runtime.runtime_api();
        let target_sample_size =
            runtime.committee_set_size(blockchain_info.finalized_hash, chain_key)?;
        let working_set_size =
            runtime.working_set_size(blockchain_info.finalized_hash, chain_key)?;

        let is_included = vrf::verify_proof_of_inclusion(
            working_set_size.into(),
            target_sample_size.into(),
            &randomness_from_two_epochs_ago,
            &attestation.proof_of_inclusion,
            &AttestorId::from_public(attestation.attestor.clone().into()),
        )?;

        if !is_included {
            info!(target: LOG_TARGET, "📝 Vrf output for {:?} is invalid ❌", attestation.attestor);
            return Err(Error::AttestorNotEligible);
        }

        info!(target: LOG_TARGET, "📝 Vrf output for {:?} is valid ✅", attestation.attestor);
        Ok(())
    }

    fn verify_attestation_data(
        &self,
        attestation: &Attestation<HashFor<B>, AccountId>,
    ) -> Result<(), Error> {
        let runtime = self.runtime.runtime_api();
        let best_hash = self.backend.blockchain().info().best_hash;
        let chain_key = attestation.attestation_data.chain_key;

        let chain_attestation_interval = runtime
            .chain_attestation_interval(self.backend.blockchain().info().best_hash, chain_key)?
            .ok_or(Error::FailedToGetAttestationInterval)?;

        let last_attestation_height =
            if let Some(digest) = runtime.last_digest(best_hash, chain_key)? {
                runtime
                    .get(best_hash, chain_key, digest)?
                    .ok_or(Error::FailedToGetLastAttestation)?
                    .header_number()
            } else {
                // If no prior attestation found, we start from 0
                0
            };

        // Attestation height should be greater than last attestation height by exactly `interval`
        if (attestation.attestation_data.header_number - last_attestation_height)
            % chain_attestation_interval
            != 0
        {
            debug!(target: LOG_TARGET, "📝 Attestation header number is invalid");
            return Err(Error::AttestationHeaderNumberInvalid);
        }

        Ok(())
    }

    /// Add attestation to round, essentially queueing it for the next round
    /// If the attestation is too old, it will be skipped
    fn add_to_round(
        &mut self,
        attestation: Attestation<HashFor<B>, AccountId>,
        block_hash: HashFor<B>,
    ) -> Result<(), Error> {
        let chain_key = attestation.attestation_data.chain_key;
        let header_number = attestation.attestation_data.header_number;
        let round = (chain_key, header_number);

        let attestor_id = attestation.attestor.clone();

        // Check last attestation that is submitted on chain. If the new one is older, skip it
        let runtime = self.runtime.runtime_api();
        let last_digest = runtime.last_digest(block_hash, chain_key)?;

        let mut target_block = 0;

        if let Some(last_digest) = last_digest {
            if let Some(last_attestation) = runtime.get(block_hash, chain_key, last_digest)? {
                let last_header = last_attestation.attestation.header_number;
                let round = (chain_key, last_header);

                let interval = runtime
                    .chain_attestation_interval(block_hash, chain_key)?
                    .ok_or(Error::FailedToGetAttestationInterval)?;

                // Skip if the attestation is too old
                if header_number <= last_header {
                    info!(target: LOG_TARGET, "📝 Attestation is too old, round {:?} already concluded on chain", round);
                    return Err(Error::AttestationTooOld);
                }

                target_block = last_header + interval;
            }
        }

        if header_number > target_block {
            info!(target: LOG_TARGET, "📝 Attestation is too early, round {:?} not yet concluded on chain", round);
            return Err(Error::AttestationTooEarly);
        }

        // Check if the chain_key exists in the block_attestations
        if let Some(attestations) = self.block_attestations.get_mut(&chain_key) {
            // Get or initialize the attestations for the header number
            let attestations_for_header = attestations
                .entry(header_number)
                .or_insert_with(HashMap::new);

            info!(
                target: LOG_TARGET,
                "📝 Attestor({:?}) voted for round {:?}", attestor_id, (chain_key, header_number)
            );

            let old_vote = attestations_for_header.insert(attestor_id.clone(), attestation);
            if old_vote.is_some() {
                info!(target: LOG_TARGET, "📝 Attestor({:?}) voted for round {:?} again", attestor_id, (chain_key, header_number));
            }
        } else {
            // Insert new attestation if it doesn't exist
            log::info!(target: LOG_TARGET, "📝 Inserting new attestation for round {:?}", round);
            let mut map = BTreeMap::new();
            let mut attestations = HashMap::new();
            attestations.insert(attestor_id, attestation.clone());
            map.insert(header_number, attestations);
            self.block_attestations.insert(chain_key, map);
        }

        Ok(())
    }

    /// In practice, this method would:
    /// 1. Gather all attesations for a round, create a BLS signature
    /// 2. Submit the inherent transaction containing the attestation
    /// 3. Flush memory
    fn try_submit_attestation(
        &mut self,
        chain_key: ChainKey,
        header_number: u64,
        block_hash: HashFor<B>,
    ) -> Result<(), Error> {
        let round = (chain_key, header_number);

        let attestations = self
            .block_attestations
            .get(&chain_key)
            .ok_or(Error::Other(
                "Error fetching attestations for chain id".to_string(),
            ))?
            .to_owned();

        let block_attestations = attestations
            .get(&header_number)
            .ok_or(Error::Other(
                "Error fetching attestation for block".to_string(),
            ))?
            .to_owned();

        let (major_digest, _) = find_major_digest::<B, AccountId>(&block_attestations);

        // Majority is more than half of the committee set size
        let runtime = self.runtime.runtime_api();
        let committee_set_size = runtime.committee_set_size(block_hash, chain_key)?;
        let threshold = calculate_threshold(committee_set_size);

        // Filter attestations by major digest
        // TODO: Can we do this in a more efficient way / place?
        let attestations = block_attestations
            .into_iter()
            .filter(|(_, attestation)| attestation.digest() == major_digest.into())
            .collect::<Vec<_>>();

        // TODO: check the list of attestations again and filter out any attestors that are not active or anymore. Based on that list, check if the threshold is reached.
        // If not, return an error and don't submit the attestation
        let attestations = attestations
            .into_iter()
            .filter(|(attestor_id, _)| {
                runtime
                    .is_attestor(block_hash, chain_key, attestor_id)
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();

        info!(
            target: LOG_TARGET,
            "📝 Majority for round {:?}, digest: {:?}, count: {:?}, threshold: {:?}",
            round,
            major_digest,
            attestations.len(),
            threshold
        );
        // If we can't find a majority voting on the same digest, we can't continue
        // Also check if the target attestation to be submitted is the same as the last attestation + interval
        // Only then we can submit the attestation
        if attestations.len() < threshold.try_into().unwrap() {
            info!(target: LOG_TARGET, "📝 Majority not reached for round {:?}", round);
            return Ok(());
        }

        let an_attestation = attestations.iter().next().cloned().unwrap();
        let chain_key = an_attestation.1.attestation_data.chain_key;
        let header_number = an_attestation.1.attestation_data.header_number;

        // check if digest exists
        let runtime = self.runtime.runtime_api();
        match runtime.contains_digest(block_hash, chain_key, major_digest.into()) {
            Ok(true) => {
                // remove from storage
                let block_attestations = self.block_attestations.get_mut(&chain_key).unwrap();
                block_attestations.remove(&header_number);
                info!(target: LOG_TARGET, "📝 Attestation is already included in runtime, need to prune from local memory here. Round: {:?}", (chain_key, header_number));
                return Ok(());
            }
            Ok(false) => {
                info!(target: LOG_TARGET, "📝 Attestation is not included in runtime, need to submit");
            }
            Err(e) => {
                error!(target: LOG_TARGET, "📝 Error while checking digest: {:?}", e);
                return Err(Error::DigestMissMatch);
            }
        };

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
            attestation: an_attestation.clone().1.attestation_data,
            signature: aggregated_signature,
            attestors,
        };

        let _ = match self.inherent_provider.0.lock() {
            Ok(mut provider) => match provider.create(attestation.clone()) {
                Ok(()) => {
                    info!(target: LOG_TARGET, "📝 Inherent created");
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
        let block_attestations = self.block_attestations.get_mut(&chain_key).unwrap();
        block_attestations.remove(&header_number);

        Ok(())
    }
}

/// Function to find the most frequently occurring digest
fn find_major_digest<B, AccountId>(attestations: &Attestations<B, AccountId>) -> (HashFor<B>, usize)
where
    B: BlockT,
    H256: From<<B as BlockT>::Hash>,
    <B as BlockT>::Hash: From<H256>,
    AccountId: Clone,
{
    let mut digest_count: HashMap<HashFor<B>, usize> = HashMap::new();
    for attestation in attestations.values() {
        let digest = attestation.digest();
        *digest_count.entry(HashFor::<B>::from(digest)).or_insert(0) += 1;
    }

    digest_count
        .into_iter()
        .max_by_key(|&(_, count)| count)
        .unwrap_or((HashFor::<B>::default(), 0))
}

/// Function to calculate the threshold for a committee set size to reach majority vote
fn calculate_threshold(committee_set_size: u32) -> u32 {
    (2 * committee_set_size + 3) / 3
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_calculate_threshold_3() {
        let committee_set_size = 3;
        let threshold = calculate_threshold(committee_set_size);
        assert_eq!(threshold, 3);
    }

    #[test]
    fn test_calculate_threshold_4() {
        let committee_set_size = 4;
        let threshold = calculate_threshold(committee_set_size);
        // TODO: why 4?
        assert_eq!(threshold, 3);
    }

    #[test]
    fn test_calculate_threshold_5() {
        let committee_set_size = 5;
        let threshold = calculate_threshold(committee_set_size);
        assert_eq!(threshold, 4);
    }

    #[test]
    fn test_calculate_threshold_10() {
        let committee_set_size = 10;
        let threshold = calculate_threshold(committee_set_size);
        assert_eq!(threshold, 7);
    }
}
