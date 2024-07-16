use attestor_primitives::bls::{Bls, CryptoScheme, WrapEncode};
use attestor_primitives::{api::AttestorApi, ChainId, SignedAttestation};
use bls_signatures::{aggregate, Serialize};
use futures::StreamExt;
use log::{debug, error, info};
use parity_scale_codec::{Codec, Decode, Encode};
use sc_client_api::{Backend, BlockBackend, HeaderBackend};
use sp_api::ProvideRuntimeApi;
use sp_consensus_babe::BabeApi;
use sp_core::{Pair, H256, U256};
use sp_inherents::CreateInherentDataProviders;
use sp_runtime::traits::{Block as BlockT, Hash as HashT, Header as HeaderT};
use std::collections::{BTreeMap, HashMap};
use std::fmt::{Debug, Display};
use std::marker::PhantomData;
use std::sync::Arc;
use supported_chains_primitives::api::SupportedChainsApi;

use crate::{Client, HashFor, LOG_TARGET};

use super::{inherent, Attestation, AttestorComms, Error, Message};

/// Gossip engine votes messages topic
pub(crate) fn votes_topic<B: BlockT>() -> B::Hash
where
    B: BlockT,
{
    <<B::Header as HeaderT>::Hashing as HashT>::hash(b"attestor-votes")
}

type BlockNumber = u64;

type Attestations<B, AccountId> = Vec<(AccountId, Attestation<HashFor<B>, AccountId>)>;

pub(crate) struct Worker<B: BlockT, RuntimeApi: ProvideRuntimeApi<B>, BE, C, CIDP, AccountId>
where
    RuntimeApi: ProvideRuntimeApi<B> + Send + Sync + 'static,
    RuntimeApi::Api: BabeApi<B>,
    RuntimeApi::Api: AttestorApi<B, HashFor<B>, AccountId>,
    RuntimeApi::Api: SupportedChainsApi<B>,
    BE: Backend<B> + 'static,
    AccountId: Clone + Display + Codec + Send + 'static + Sync + Debug + Into<[u8; 32]> + PartialEq,
{
    /// Best attestation we have in the cache (latest)
    #[allow(dead_code)]
    pub best_attestation: Option<SignedAttestation<HashFor<B>, AccountId>>,

    /// communication (created once, but returned and reused if worker is restarted/reinitialized)
    pub comms: AttestorComms<B, AccountId, RuntimeApi, BE>,

    /// runtime api access
    pub runtime: Arc<RuntimeApi>,

    pub client: Arc<C>,
    /// Client Backend
    pub backend: Arc<BE>,

    /// Block attestations. Maps a blocknumber to a list of actual attestations, not digests
    pub block_attestations: BTreeMap<ChainId, BTreeMap<BlockNumber, Attestations<B, AccountId>>>,

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
    BE: Backend<B> + 'static,
    AccountId: Clone + Display + Codec + Send + 'static + Sync + Debug + Into<[u8; 32]> + PartialEq,
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
    BE: Backend<B>,
    C: Client<B, BE> + BlockBackend<B>,
    CIDP: CreateInherentDataProviders<B, ()> + 'static,
    H256: From<<B as BlockT>::Hash>,
    <B as BlockT>::Hash: From<H256>,
    <<B as BlockT>::Header as HeaderT>::Number: Into<u64>,
    AccountId: Clone + Display + Codec + Send + 'static + Sync + Debug + Into<[u8; 32]> + PartialEq,
{
    pub fn new(params: WorkerParams<B, RA, BE, C, CIDP, AccountId>) -> Self {
        Worker {
            best_attestation: None,
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
                                let chain_id = attestation.attestation_data.chain_id;
                                let header_number = attestation.attestation_data.header_number;

                                let round = (chain_id, header_number);
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

        // First check if we have this one already in our cache
        if self.check_attestation_in_cache(&attestation, block_hash) {
            info!(target: LOG_TARGET, "📝 Already have this attestation in cache");
            return Ok(()); // we already have this attestation
        }

        // Verify the attestation data
        self.verify_attestation_data(&attestation)?;

        // Verify the VRF output
        self.verify_vrf(&attestation)?;

        let submitable_attestations = self.add_to_round(&attestation, block_hash)?;

        // conclude round
        // create the inherent
        if let Some(submitable_attestations) = submitable_attestations {
            info!(target: LOG_TARGET, "📝 Should be able to create the inherent now and submit the vote");
            self.submit_attestation(submitable_attestations, block_hash)?;
        } else {
            info!(target: LOG_TARGET,"📝 cannot submit attestation");
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

        let runtime = self.runtime.runtime_api();
        let is_attestor =
            runtime.is_attestor(blockchain_info.best_hash, &attestation.attestor.clone())?;
        debug!(target: LOG_TARGET, "📝 {} Is attestor {}", attestation.attestor, is_attestor);

        if !is_attestor {
            return Err(Error::NotAnAttestor);
        }

        let last_digest = runtime.last_digest(
            blockchain_info.best_hash,
            attestation.attestation_data.chain_id,
        )?;

        let prev_digest = attestation.attestation_data.prev_digest;

        if last_digest != prev_digest {
            debug!(target: LOG_TARGET, "📝 last digest: {:?}, attestation digest {:?}", last_digest, prev_digest);

            return Err(Error::DigestMissMatch);
        }

        // Get the vrf at 2 epochs ago
        // If not 2 epochs have passed, return an error
        let runtime = self.runtime.runtime_api();
        let config = runtime.configuration(blockchain_info.best_hash)?;

        let target_epoch_block: u64 = match blockchain_info
            .best_number
            .into()
            .checked_sub(config.epoch_length * 2)
        {
            Some(result) => result,
            None => {
                info!("We cannot go back 2 epoch yet, that means we just need to fetch the randomness from current epoch");
                blockchain_info.best_number.into()
            }
        };

        debug!(target: LOG_TARGET, "📝 target block to fetch vrf from: {:?}", target_epoch_block);

        let target_epoch_hash = self
            .client
            .block_hash((target_epoch_block as u32).into())
            .ok()
            .flatten()
            .expect("Target block exists; qed");

        let runtime = self.runtime.runtime_api();
        let vrf_target_epoch: sp_consensus_babe::Epoch =
            runtime.current_epoch(target_epoch_hash)?;

        // Get the vrf for the attestation that was submitted
        let runtime = self.runtime.runtime_api();
        let vrf_epoch: sp_consensus_babe::Epoch =
            runtime.current_epoch(attestation.vrf_output.block_hash.into())?;

        // Format the randomness as a number
        let randomness_u256 = U256::from_little_endian(&vrf_epoch.randomness);

        if attestation.vrf_output.vrf_number >= randomness_u256 {
            debug!(target: LOG_TARGET, "📝 Vrf output for {:?} is valid ✅", attestation.attestor);

            // now check if the number that the attestor generated actually is correct ?
            // otherwise, slash the attestor
            let public_key =
                sp_core::sr25519::Public::from_raw(attestation.attestor.clone().into());

            let is_valid = sp_core::sr25519::Pair::verify(
                &attestation.vrf_output.signature,
                vrf_target_epoch.randomness,
                &public_key,
            );

            debug!(target: LOG_TARGET, "📝 Vrf output for {:?} signature is valid: {is_valid}", attestation.attestor);
        } else {
            debug!(target: LOG_TARGET, "📝 Vrf output for {:?} is invalid ❌", attestation.attestor);
        }

        Ok(())
    }

    fn verify_attestation_data(
        &self,
        attestation: &Attestation<HashFor<B>, AccountId>,
    ) -> Result<(), Error> {
        let runtime = self.runtime.runtime_api();
        let chain_attestation_interval = runtime
            .chain_attestation_interval(
                self.backend.blockchain().info().best_hash,
                attestation.attestation_data.chain_id,
            )?
            .ok_or(Error::AttestationHeaderNumberInvalid)?;

        // Attestation block number mod chain_attestation_interval should be 0
        if attestation.attestation_data.header_number % chain_attestation_interval != 0 {
            debug!(target: LOG_TARGET, "📝 Attestation header number is invalid");
            return Err(Error::AttestationToEarly);
        }

        Ok(())
    }

    /// Add attestation to round, essentially queueing it for the next round
    /// If the attestation is too old, it will be skipped
    fn add_to_round(
        &mut self,
        attestation: &Attestation<HashFor<B>, AccountId>,
        block_hash: HashFor<B>,
    ) -> Result<Option<Attestations<B, AccountId>>, Error> {
        let chain_id = attestation.attestation_data.chain_id;
        let header_number = attestation.attestation_data.header_number;
        let round = (chain_id, header_number);

        let attestor_id = attestation.attestor.clone();

        // Check last attestation that is submitted on chain. If the new one is older, skip it
        let runtime = self.runtime.runtime_api();
        let last_digest = runtime.last_digest(block_hash, chain_id)?;

        let mut target_block = 0;
        if let Some(last_digest) = last_digest {
            let last_attestation = runtime.get(block_hash, chain_id, last_digest)?;

            if let Some(last_attestation) = last_attestation {
                let last_header = last_attestation.attestation.header_number;
                let round = (chain_id, last_header);

                let interval = runtime
                    .chain_attestation_interval(block_hash, chain_id)?
                    .ok_or(Error::AttestationHeaderNumberInvalid)?;
                target_block = last_header + interval;
                // skip if the attestation is too old
                if header_number < last_header {
                    info!(target: LOG_TARGET, "📝 Attestation is too old, round {:?} already concluded on chain", round);
                    return Ok(None);
                }
            }
        }

        // Check if the chain_id exists in the block_attestations
        if let Some(attestations) = self.block_attestations.get_mut(&chain_id) {
            // Get or initialize the attestations for the header number
            let attestations_for_header =
                attestations.entry(header_number).or_insert_with(Vec::new);

            info!(
                target: LOG_TARGET,
                "📝 Attestor({:?}) voted for round {:?}", attestor_id, (chain_id, header_number)
            );
            // insert the attestation into the attestations for the header number
            attestations_for_header.push((attestor_id, attestation.clone()));

            // If majority is reached, return a list of attestations to be submitted
            let (major_digest, major_count) =
                find_major_digest::<B, AccountId>(attestations_for_header);

            // TODO: should be per chain id
            let runtime = self.runtime.runtime_api();
            let threshold = runtime.comittee_set_size(block_hash).unwrap_or(0);

            info!(
                target: LOG_TARGET,
                "📝 Majority for round {:?}, digest: {:?}, count: {:?}, threshold: {:?}",
                round,
                major_digest,
                major_count,
                threshold
            );
            // If we can't find a majority voting on the same digest, we can't continue
            // Also check if the target attestation to be submitted is the same as the last attestation + interval
            // Only then we can submit the attestation
            if (major_count as u32) >= threshold && target_block == header_number {
                info!(target: LOG_TARGET, "📝 Majority found for round {:?}", round);
                return Ok(Some(attestations_for_header.clone()));
            }
        } else {
            // Insert new attestation if it doesn't exist
            log::info!(target: LOG_TARGET, "📝 Inserting new attestation for round {:?}", round);
            let mut map = BTreeMap::new();
            map.insert(header_number, vec![(attestor_id, attestation.clone())]);
            self.block_attestations.insert(chain_id, map);
        }

        Ok(None)
    }

    /// In practice, this method would:
    /// 1. Gather all attesations for a round, create a BLS signature
    /// 2. Submit the inherent transaction containing the attestation
    /// 3. Flush memory
    fn submit_attestation(
        &mut self,
        attestations: Attestations<B, AccountId>,
        block_hash: HashFor<B>,
    ) -> Result<(), Error> {
        let an_attestation = attestations.iter().next().cloned().unwrap();
        let chain_id = an_attestation.1.attestation_data.chain_id;
        let header_number = an_attestation.1.attestation_data.header_number;

        let (major_digest, _) = find_major_digest::<B, AccountId>(&attestations);

        // check if digest exists
        let runtime = self.runtime.runtime_api();
        match runtime.contains_digest(block_hash, chain_id, major_digest.into()) {
            Ok(true) => {
                // remove from storage
                let block_attestations = self.block_attestations.get_mut(&chain_id).unwrap();
                block_attestations.remove(&header_number);
                info!(target: LOG_TARGET, "📝 Attestation is already included in runtime, need to prune from local memory here. Round: {:?}", (chain_id, header_number));
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

        // Filter attestations by major digest
        // TODO: Can we do this in a more efficient way / place?
        let attestations = attestations
            .into_iter()
            .filter(|(_, attestation)| attestation.digest() == major_digest.into())
            .collect::<Vec<_>>();

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
            // digest: major_digest,
            attestors,
        };

        // Some safety check
        if self.best_attestation == Some(attestation.clone()) {
            info!(target: LOG_TARGET, "📝 Best attestation already submitted");
            return Err(Error::ErrorCreatingInherent);
        }

        let _ = match self.inherent_provider.0.lock() {
            Ok(mut provider) => provider.create(attestation.clone()),
            Err(e) => {
                error!("error acquiring lock, {:?}", e);
                Ok(())
            }
        };

        // Update best attestation
        self.best_attestation = Some(attestation);

        // Flush memory
        let block_attestations = self.block_attestations.get_mut(&chain_id).unwrap();
        block_attestations.remove(&header_number);

        Ok(())
    }

    /// Check if the attestation is already in the cache
    /// This is useful to avoid submitting the same attestation multiple times
    /// It also checks if it's already included in the runtime
    pub fn check_attestation_in_cache(
        &self,
        attestation: &Attestation<HashFor<B>, AccountId>,
        block_hash: HashFor<B>,
    ) -> bool {
        let chain_id = attestation.attestation_data.chain_id;
        let header_number = attestation.attestation_data.header_number;

        let attestations = self.block_attestations.get(&chain_id);
        if attestations.is_none() {
            return false;
        }

        let attestations = attestations.unwrap().get(&header_number);
        if attestations.is_none() {
            return false;
        }

        if attestations
            .unwrap()
            .iter()
            .any(|(_, att)| att == attestation)
        {
            info!(target: LOG_TARGET, "📝 Attestation is already in cache, no need to do anything here. Round: {:?}", (chain_id, header_number));
            return true;
        }

        // check if attestor already pushed a similar message
        if attestations
            .unwrap()
            .iter()
            .any(|(attestor, _)| attestor == &attestation.attestor)
        {
            info!(target: LOG_TARGET, "📝 Attestor already submitted a similar attestation, no need to do anything here. Round: {:?}", (chain_id, header_number));
            return true;
        }

        let runtime = self.runtime.runtime_api();
        match runtime.contains_digest(block_hash, chain_id, attestation.digest()) {
            Ok(true) => {
                info!(target: LOG_TARGET, "📝 Attestation is already included in runtime, no need to do anything here. Round: {:?}", (chain_id, header_number));
                true
            }
            Ok(false) => {
                info!(target: LOG_TARGET, "📝 Attestation is not included in runtime, need to proceed");
                false
            }
            Err(e) => {
                error!(target: LOG_TARGET, "📝 Error while checking digest: {:?}", e);
                false
            }
        }
    }
}

/// Function to find the most frequently occurring digest
fn find_major_digest<B, AccountId>(
    attestations: &Vec<(AccountId, Attestation<HashFor<B>, AccountId>)>,
) -> (HashFor<B>, usize)
where
    B: BlockT,
    H256: From<<B as BlockT>::Hash>,
    <B as BlockT>::Hash: From<H256>,
    AccountId: Clone,
{
    let mut digest_count: HashMap<HashFor<B>, usize> = HashMap::new();
    for (_, attestation) in attestations {
        let digest = attestation.digest();
        *digest_count.entry(HashFor::<B>::from(digest)).or_insert(0) += 1;
    }

    digest_count
        .into_iter()
        .max_by_key(|&(_, count)| count)
        .unwrap_or((HashFor::<B>::default(), 0))
}
