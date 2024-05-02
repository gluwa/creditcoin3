use attestor_primitives::bls::{Bls, CryptoScheme, WrapEncode};
use attestor_primitives::{api::AttestorApi, Digest, SignedAttestation};
use bls_signatures::{aggregate, Serialize};
use futures::StreamExt;
use log::{error, info};
use parity_scale_codec::{Codec, Decode, Encode};
use sc_client_api::{Backend, BlockBackend, HeaderBackend};
use sp_api::ProvideRuntimeApi;
use sp_consensus_babe::BabeApi;
use sp_core::{Pair, H256, U256};
use sp_inherents::CreateInherentDataProviders;
use sp_runtime::{
    traits::{Block as BlockT, Hash as HashT, Header as HeaderT},
    AccountId32,
};
use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use crate::{Client, HashFor, LOG_TARGET};

use super::{Attestation, AttestorComms, AttestorId, Error, Message};

const THRESHOLD: usize = 2; // You can set this to any appropriate threshold value

/// Gossip engine votes messages topic
pub(crate) fn votes_topic<B: BlockT>() -> B::Hash
where
    B: BlockT,
{
    <<B::Header as HeaderT>::Hashing as HashT>::hash(b"attestor-votes")
}

// Should be ChainID
type ChainId = u8;

type BlockNumber = u64;

pub(crate) struct Worker<B: BlockT, RuntimeApi: ProvideRuntimeApi<B>, BE, C, CIDP, AccountId> {
    /// Best attestation we have in the cache (latest)
    #[allow(dead_code)]
    pub best_attestation: Option<Attestation<HashFor<B>, AccountId>>,

    /// communication (created once, but returned and reused if worker is restarted/reinitialized)
    pub comms: AttestorComms<B, AccountId>,

    /// runtime api access
    pub runtime: Arc<RuntimeApi>,

    pub client: Arc<C>,
    /// Client Backend
    pub backend: Arc<BE>,

    /// Block attestations. Maps a blocknumber to a list of valid attestations
    pub block_attestations: HashMap<(ChainId, BlockNumber), Vec<(AttestorId, Digest)>>,

    pub block_attestations_raw:
        HashMap<(ChainId, BlockNumber), Vec<(AccountId, Attestation<HashFor<B>, AccountId>)>>,
    /// Inherent data providers
    #[allow(dead_code)]
    pub create_inherent_data_providers: CIDP,

    pub inherent_provider: Arc<Mutex<crate::inherent::Provider<HashFor<B>>>>,

    pub _phantom: PhantomData<AccountId>,
}

pub(crate) struct WorkerParams<B: BlockT, RuntimeApi: ProvideRuntimeApi<B>, BE, C, CIDP, AccountId>
{
    /// communication (created once, but returned and reused if worker is restarted/reinitialized)
    pub comms: AttestorComms<B, AccountId>,

    /// runtime api access
    pub runtime: Arc<RuntimeApi>,

    pub client: Arc<C>,

    /// Inherent data providers
    pub create_inherent_data_providers: CIDP,

    /// Client Backend
    pub backend: Arc<BE>,

    pub inherent_provider: Arc<Mutex<crate::inherent::Provider<HashFor<B>>>>,

    pub _phantom: PhantomData<AccountId>,
}

impl<B: BlockT, RA: ProvideRuntimeApi<B>, BE, C, CIDP, AccountId>
    Worker<B, RA, BE, C, CIDP, AccountId>
where
    B: BlockT,
    RA: ProvideRuntimeApi<B> + Send + Sync + 'static,
    RA::Api: BabeApi<B>,
    RA::Api: AttestorApi<B, AccountId>,
    BE: Backend<B>,
    C: Client<B, BE> + BlockBackend<B>,
    CIDP: CreateInherentDataProviders<B, ()> + 'static,
    H256: From<<B as BlockT>::Hash>,
    <B as BlockT>::Hash: From<H256>,
    <<B as BlockT>::Header as HeaderT>::Number: Into<u64>,
    AccountId: Clone + Display + Codec + Send + 'static + Sync + Debug + Into<[u8; 32]>,
{
    pub fn new(params: WorkerParams<B, RA, BE, C, CIDP, AccountId>) -> Self {
        Worker {
            best_attestation: None,
            comms: params.comms,
            runtime: params.runtime,
            client: params.client,
            create_inherent_data_providers: params.create_inherent_data_providers,
            block_attestations: HashMap::new(),
            block_attestations_raw: HashMap::new(),
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

        // self.process_new_state();

        // Main process loop
        loop {
            // Mutable reference used to drive the gossip engine.
            let mut gossip_engine = &mut self.comms.gossip_engine;
            let message_stream = &mut self.comms.gossip_report_stream;

            // Wait for, and handle external events.
            // The branches below only change 'state', actual voting happens afterwards,
            // based on the new resulting 'state'.

            // In branch one: process a vote (receiver node)
            // In branch two: propagate a vote
            // ...

            futures::select_biased! {
                    // Make sure to pump gossip engine.
                    _ = gossip_engine => {
                        break Error::GossipEngineExited;
                    },
                    // PROCESS HANDLER
                    // Finally process incoming votes.
                    vote = votes.next() => {
                        // If this node is a validator
                        // validate the vote
                        // if valid store in memory
                        // if enough votes -> process
                        // ....

                        if let Some(vote) = vote {
                            match self.triage_message(vote).await {
                                Ok(()) => {
                                    info!(target: LOG_TARGET, "📝 Got a valid gossiped message");
                                },
                                Err(e) => {
                                    error!(target: LOG_TARGET, "📝 Got error for message err: {:?}", e);
                                }
                            }
                        } else {
                            break Error::GossipEngineExited;
                        }
                    },
                    // GOSSIP HANDLER
                    message = message_stream.next() => {
                        if let Some(message) = message {
                            let topic = votes_topic::<B>();
                            info!(target: LOG_TARGET, "📝 📝 Got message to gossip {:?}, on topic: {:?}", message, topic);
                            gossip_engine.gossip_message(
                                topic,
                                message.encode(),
                                false,
                            );
                        }
            }}
        }
    }

    async fn triage_message(&mut self, message: Message<B, AccountId>) -> Result<(), Error> {
        match message {
            Message::Attestation(attestation) => {
                self.verify_vrf(&attestation)?;

                if self.add_to_round(&attestation) {
                    // conclude round
                    // create the inherent
                    let _best_block_hash = self.backend.blockchain().info().best_hash;

                    if let Some(inherent) = self.submit_attestation(attestation) {
                        info!(target: LOG_TARGET, "📝 Should be able to create the inherent now and submit the vote");
                        let _ = match self.inherent_provider.lock() {
                            Ok(mut provider) => provider.create(inherent),
                            Err(e) => {
                                error!("error acquiring lock, {:?}", e);
                                Ok(())
                            }
                        };
                    } else {
                        info!(target: LOG_TARGET,"📝 cannot submit attestation");
                    }
                } else {
                    info!(target: LOG_TARGET, "📝 Received a valid vote, need more in order to conclude the round...");
                }
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

        let runtime = self.runtime.runtime_api();
        let is_attestor =
            runtime.is_attestor(blockchain_info.best_hash, &attestation.attestor.clone())?;
        info!(target: LOG_TARGET, "📝 {} Is attestor {}", attestation.attestor, is_attestor);

        if !is_attestor {
            return Err(Error::NotAnAttestor);
        }

        let last_digest = runtime
            .last_digest(
                blockchain_info.best_hash,
                attestation.attestation_data.chain_id,
            )?
            .unwrap_or(H256::zero());

        if last_digest != attestation.attestation_data.prev_digest.into() {
            info!(target: LOG_TARGET, "📝 last digest: {:?}, attestation digest {:?}", last_digest, attestation.attestation_data.prev_digest);

            return Err(Error::DigestMissMatch);
        }

        // Get the vrf at 2 epochs ago
        let runtime = self.runtime.runtime_api();
        let config = runtime.configuration(blockchain_info.best_hash)?;
        let target_epoch_block: u64 =
            blockchain_info.best_number.into() - (config.epoch_length * 2);
        info!(target: LOG_TARGET, "📝 target block to fetch vrf from: {:?}", target_epoch_block);

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
            info!(target: LOG_TARGET, "📝 Vrf output for {:?} is valid ✅", attestation.attestor);

            // now check if the number that the attestor generated actually is correct ?
            // otherwise, slash the attestor
            let public_key =
                sp_core::sr25519::Public::from_raw(attestation.attestor.clone().into());

            let is_valid = sp_core::sr25519::Pair::verify(
                &attestation.vrf_output.signature,
                vrf_target_epoch.randomness,
                &public_key,
            );

            info!(target: LOG_TARGET, "📝 Vrf output for {:?} signature is valid: {is_valid}", attestation.attestor);
        } else {
            info!(target: LOG_TARGET, "📝 Vrf output for {:?} is invalid ❌", attestation.attestor);
        }

        Ok(())
    }

    /// Add attestation to round, returns if we need to conclude the round or not
    fn add_to_round(&mut self, attestation: &Attestation<HashFor<B>, AccountId>) -> bool {
        // Hash the attestation data
        let digest = attestation.attestation_data.digest();

        let k = (
            attestation.attestation_data.chain_id,
            attestation.attestation_data.header_number,
        );

        if let Some(raw_attestations) = self.block_attestations_raw.get_mut(&k) {
            raw_attestations.push((attestation.attestor.clone(), attestation.clone()));
        } else {
            self.block_attestations_raw
                .insert(k, vec![(attestation.attestor.clone(), attestation.clone())]);
        };

        let attestor_id = AttestorId(AccountId32::from(attestation.attestor.clone().into()));

        let exceed_threshold = if let Some(attestations) = self.block_attestations.get_mut(&k) {
            attestations.push((attestor_id, digest));
            attestations.len() >= THRESHOLD
        } else {
            self.block_attestations
                .insert(k, vec![(attestor_id, digest)]);
            false // Newly inserted, so it cannot exceed the threshold yet
        };

        // If exceeds threshold check if we can have a majority of attestors that have pushed the same attestation
        exceed_threshold
    }

    /// In practice, this method would:
    /// 1. Gather all attesations for a round, create a BLS signature
    /// 2. Submit the inherent transaction containing the attestation
    /// 3. Flush memory
    fn submit_attestation(
        &mut self,
        attestation: Attestation<HashFor<B>, AccountId>,
    ) -> Option<SignedAttestation<HashFor<B>>> {
        let k = (
            attestation.attestation_data.chain_id,
            attestation.attestation_data.header_number,
        );

        let attestations = self.block_attestations.get(&k).unwrap();
        let (major_digest, major_count) = find_major_digest(attestations);

        // here goes bls
        // contains attestorid, and attestation itself.
        let raw_attestations = self.block_attestations_raw.get(&k).unwrap();

        // will be needed for later verification
        // let messages = raw_attestations
        //     .iter()
        //     .map(|(_, attestation)| attestation.attestation_data.serialize())
        //     .collect::<Vec<Vec<u8>>>();

        // retrieve wrapped bls signatures
        let signatures = raw_attestations
            .iter()
            .map(|(_, attestations)| attestations.signature_bls.clone())
            .collect::<Vec<<Bls as CryptoScheme>::Signature>>();

        // will be needed for later verification
        // let public_keys = raw_attestations.iter().map(|(attestor, _)| attestor.0.clone()).collect::<Vec<[u8; 32]>>();

        // retrieve inner bls signature
        let sigs = signatures
            .iter()
            .map(|WrapEncode(sig)| *sig)
            .collect::<Vec<_>>();

        let aggregated_signature = aggregate(&sigs[..]).expect("Failed to aggregate signatures");

        if major_count >= THRESHOLD {
            let bls = aggregated_signature; // Placeholder for BLS signature computation
            let res = Some(SignedAttestation {
                attestation_data: attestation.clone().attestation_data,
                signature: bls.as_bytes()[..96].try_into().unwrap(),
                digest: major_digest,
            });

            // Update best attestation
            self.best_attestation = Some(attestation);

            // Flush memory
            self.block_attestations.remove(&k);
            self.block_attestations_raw.remove(&k);

            res
        } else {
            // Handle case where no single digest is dominant
            // e.g., log an error, alert, etc.
            // Optionally return a list of incorrect attestors
            None
        }
    }
}

/// Function to find the most frequently occurring digest
fn find_major_digest(attestations: &Vec<(AttestorId, Digest)>) -> (Digest, usize) {
    let mut digest_count: HashMap<Digest, usize> = HashMap::new();
    for (_, digest) in attestations {
        *digest_count.entry(*digest).or_insert(0) += 1;
    }

    digest_count
        .into_iter()
        .max_by_key(|&(_, count)| count)
        .unwrap_or((H256::zero(), 0))
}
