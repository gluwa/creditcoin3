use futures::StreamExt;
use log::{error, info};
use parity_scale_codec::{Decode, Encode};
use sc_client_api::{Backend, BlockBackend, HeaderBackend};
use sp_api::ProvideRuntimeApi;
use sp_consensus_babe::BabeApi;
use sp_core::H256;
use sp_inherents::{CreateInherentDataProviders, InherentDataProvider};
use sp_runtime::traits::{Block as BlockT, Hash, Header as HeaderT};
use std::collections::HashMap;
use std::{marker::PhantomData, sync::Arc};

use crate::{Client, HashFor, LOG_TARGET};

use super::{Attestation, AttestorComms, Error, Message};

const THRESHOLD: usize = 3; // You can set this to any appropriate threshold value

/// Gossip engine votes messages topic
pub(crate) fn votes_topic<B: BlockT>() -> B::Hash
where
    B: BlockT,
{
    <<B::Header as HeaderT>::Hashing as Hash>::hash(b"attestor-votes")
}

// Should be ChainID
type Round = u64;

type BlockNumber = u64;

pub(crate) struct Worker<B: BlockT, RuntimeApi: ProvideRuntimeApi<B>, BE, C, CIDP> {
    /// Best attestation we have in the cache (latest)
    #[allow(dead_code)]
    pub best_attestation: Option<Attestation<HashFor<B>>>,

    /// communication (created once, but returned and reused if worker is restarted/reinitialized)
    pub comms: AttestorComms<B>,

    /// runtime api access
    pub runtime: Arc<RuntimeApi>,

    pub client: Arc<C>,
    /// Client Backend
    pub backend: Arc<BE>,

    /// Block attestations. Maps a blocknumber to a list of valid attestations
    pub block_attestations: HashMap<(Round, BlockNumber), Vec<Attestation<HashFor<B>>>>,

    /// Inherent data providers
    pub create_inherent_data_providers: CIDP,
}

pub(crate) struct WorkerParams<B: BlockT, RuntimeApi: ProvideRuntimeApi<B>, BE, C, CIDP> {
    /// communication (created once, but returned and reused if worker is restarted/reinitialized)
    pub comms: AttestorComms<B>,

    /// runtime api access
    pub runtime: Arc<RuntimeApi>,

    pub client: Arc<C>,

    /// Inherent data providers
    pub create_inherent_data_providers: CIDP,

    /// Client Backend
    pub backend: Arc<BE>,
}

impl<B: BlockT, RA: ProvideRuntimeApi<B>, BE, C, CIDP> Worker<B, RA, BE, C, CIDP>
where
    B: BlockT,
    RA: ProvideRuntimeApi<B> + Send + Sync + 'static,
    RA::Api: BabeApi<B>,
    BE: Backend<B>,
    C: Client<B, BE> + BlockBackend<B>,
    CIDP: CreateInherentDataProviders<B, ()> + 'static,
    H256: From<<B as BlockT>::Hash>,
    <B as BlockT>::Hash: From<H256>,
    // H: std::hash::Hash + Serialize + Debug,
{
    pub fn new(params: WorkerParams<B, RA, BE, C, CIDP>) -> Self {
        Worker {
            best_attestation: None,
            comms: params.comms,
            runtime: params.runtime,
            client: params.client,
            create_inherent_data_providers: params.create_inherent_data_providers,
            block_attestations: HashMap::new(),
            backend: params.backend,
        }
    }

    pub async fn start(mut self) -> Error {
        let mut votes = Box::pin(
            self.comms
                .gossip_engine
                .messages_for(votes_topic::<B>())
                .filter_map(|notification| async move {
                    let message = Message::<B>::decode(&mut &notification.message[..])
                        .ok()
                        .map(|m| m);

                    message
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
                            info!(target: LOG_TARGET, "📝 GOT A VOTE: {:?}", vote);

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

    async fn triage_message(&mut self, message: Message<B>) -> Result<(), Error> {
        match message {
            Message::Attestation(attestation) => {
                self.verify_vrf(&attestation)?;

                if self.add_to_round(attestation.clone()) {
                    // conclude round
                    // create the inherent
                    let best_block_hash = self.backend.blockchain().info().best_hash;

                    // Somehow get the current block?
                    info!(target: LOG_TARGET, "📝 Should be able to create the inherent now and submit the vote");
                    // self.create_inherent_data(best_block_hash, &attestation)
                    //     .await?;
                    // flush round
                } else {
                    info!(target: LOG_TARGET, "📝 Received a valid vote, need more in order to conclude the round...");
                }
            }
        }

        Ok(())
    }

    fn verify_vrf(&self, attestation: &Attestation<HashFor<B>>) -> Result<(), Error> {
        // check if the attestation vrf output is submitted correctly and is eligible for attesting
        let runtime = self.runtime.runtime_api();

        let config = runtime.configuration(attestation.vrf_output.block_hash.into())?;
        info!(target: LOG_TARGET, "📝 Epoch config: {:?}", config);

        let vrf_epoch = runtime.current_epoch(attestation.vrf_output.block_hash.into())?;
        info!(target: LOG_TARGET, "📝 Vrf epoch: {:?}", vrf_epoch);

        let client = self.client.clone();

        let _hash_at_height = client
            .block_hash((attestation.header_number as u32).into())
            .ok()
            .flatten()
            .expect("Genesis block exists; qed");

        let _randomness = vrf_epoch.randomness;

        Ok(())
    }

    /// Add attestation to round, returns if we need to conclude the round or not
    fn add_to_round(&mut self, attestation: Attestation<HashFor<B>>) -> bool {
        let k = (attestation.round, attestation.header_number);

        let exceed_threshold = if let Some(attestations) = self.block_attestations.get_mut(&k) {
            attestations.push(attestation);
            attestations.len() >= THRESHOLD
        } else {
            self.block_attestations.insert(k, vec![attestation]);
            false // Newly inserted, so it cannot exceed the threshold yet
        };

        exceed_threshold
    }

    /// In practice, this method would:
    /// 1. Gather all attesations for a round, create a BLS signature
    /// 2. Check if the current validator where this is running is allowed to create the next block
    /// 3. If yes, submit the inherent transaction containing the attestation
    /// 4. Flush memory
    fn _submit_attestation(&mut self, attestation: Attestation<HashFor<B>>) -> Result<(), Error> {
        self.best_attestation = Some(attestation);

        Ok(())
    }

    // pub async fn create_inherent_data(
    //     &self,
    //     parent: B::Hash,
    //     attestation: &Attestation<HashFor<B>>,
    // ) -> Result<(), Error> {
    //     let data = AttestationInherent {
    //         attestation: attestation.clone(),
    //         signatures: vec![],
    //     };

    //     let inherent_data_providers = self
    //         .create_inherent_data_providers
    //         .create_inherent_data_providers(parent, data)
    //         .await
    //         .map_err(|e| {
    //             error!("Error creating inherent data: {e}");
    //             Error::ErrorCreatingInherent
    //         })?;

    //     let inherent_data = inherent_data_providers
    //         .create_inherent_data()
    //         .await
    //         .map_err(|e| {
    //             error!(
    //                 target: LOG_TARGET,
    //                 "Failed to create inherent data.",
    //             );
    //             Error::ErrorCreatingInherent
    //         })?;

    //     // WHAT HAPPENS WITH INHERENT DATA

    //     // log::info!(target: LOG_TARGET, "inherent data: {:?}", x.);

    //     Ok(())
    // }
}
