use futures::StreamExt;
use parity_scale_codec::{Decode, Encode};
use sc_client_api::{Backend, BlockBackend};
use sp_api::ProvideRuntimeApi;
use sp_consensus_babe::BabeApi;
use sp_core::H256;
use sp_runtime::traits::{Block as BlockT, Hash, Header as HeaderT};
use std::{marker::PhantomData, sync::Arc};

use crate::{Client, HashFor};

use super::{Attestation, AttestorComms, Error, Message};

/// Gossip engine votes messages topic
pub(crate) fn votes_topic<B: BlockT>() -> B::Hash
where
    B: BlockT,
{
    <<B::Header as HeaderT>::Hashing as Hash>::hash(b"attestor-votes")
}

pub(crate) struct Worker<B: BlockT, RuntimeApi: ProvideRuntimeApi<B>, BE, C> {
    /// Best attestation we have in the cache (latest)
    #[allow(dead_code)]
    pub best_attestation: Option<Attestation<HashFor<B>>>,

    /// communication (created once, but returned and reused if worker is restarted/reinitialized)
    pub comms: AttestorComms<B>,

    /// runtime api access
    pub runtime: Arc<RuntimeApi>,

    pub client: Arc<C>,
    /// Client Backend
    pub backend: PhantomData<BE>,
}

impl<B: BlockT, RA: ProvideRuntimeApi<B>, BE, C> Worker<B, RA, BE, C>
where
    B: BlockT,
    RA: ProvideRuntimeApi<B> + Send + Sync + 'static,
    RA::Api: BabeApi<B>,
    BE: Backend<B>,
    C: Client<B, BE> + BlockBackend<B>,
    H256: From<<B as BlockT>::Hash>,
    <B as BlockT>::Hash: From<H256>,
    // H: std::hash::Hash + Serialize + Debug,
{
    pub fn new(comms: AttestorComms<B>, runtime: Arc<RA>, client: Arc<C>) -> Self {
        Worker {
            best_attestation: None,
            comms,
            runtime,
            client,
            backend: PhantomData,
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
                            log::info!(target: "attestor-gossip", "GOT A VOTE: {:?}", vote);

                            match self.triage_message(&vote) {
                                Ok(()) => {
                                    log::info!(target: "attestor-gossip", "Got a valid gossiped message {:?}", vote);
                                    // TODO: store in memmory
                                },
                                Err(e) => {
                                    log::error!(target: "attestor-gossip", "Got error for message err: {:?}", e);
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
                            log::info!(target: "attestor-gossip", "Got message to gossip {:?}, on topic: {:?}", message, topic);
                            gossip_engine.gossip_message(
                                topic,
                                message.encode(),
                                false,
                            );
                        }
            }}
        }
    }

    fn triage_message(&mut self, message: &Message<B>) -> Result<(), Error> {
        match message {
            Message::Attestation(attestation) => {
                self.verify_vrf(attestation)?;

                let topic = votes_topic::<B>();

                self.comms
                    .gossip_engine
                    .gossip_message(topic, message.encode(), false);
            }
        }

        Ok(())
    }

    fn verify_vrf(&self, attestation: &Attestation<HashFor<B>>) -> Result<(), Error> {
        // check if the attestation vrf output is submitted correctly and is eligible for attesting
        let runtime = self.runtime.runtime_api();

        let config = runtime.configuration(attestation.vrf_output.block_hash.into())?;
        log::info!(target: "attestor-gossip", "Epoch config: {:?}", config);

        let vrf_epoch = runtime.current_epoch(attestation.vrf_output.block_hash.into())?;
        log::info!(target: "attestor-gossip", "Vrf epoch: {:?}", vrf_epoch);

        let client = self.client.clone();

        let _hash_at_height = client
            .block_hash((attestation.header_number as u32).into())
            .ok()
            .flatten()
            .expect("Genesis block exists; qed");

        let _randomness = vrf_epoch.randomness;

        Ok(())
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
}
