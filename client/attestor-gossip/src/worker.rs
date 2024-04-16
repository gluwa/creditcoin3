use futures::StreamExt;
use parity_scale_codec::{Decode, Encode};
use sc_client_api::BlockBackend;
use sp_api::ProvideRuntimeApi;
use sp_consensus_babe::BabeApi;
use sp_core::{H256, U256};
use sp_runtime::traits::{Block as BlockT, Hash, Header as HeaderT, NumberFor};
use std::sync::Arc;

use crate::HashFor;

use super::{Attestation, AttestorComms, Error, Message};

/// Gossip engine votes messages topic
pub(crate) fn votes_topic<B: BlockT>() -> B::Hash
where
    B: BlockT,
{
    <<B::Header as HeaderT>::Hashing as Hash>::hash(b"attestor-votes")
}

pub(crate) struct Worker<B: BlockT, RuntimeApi: ProvideRuntimeApi<B>> {
    /// Best attestation we have in the cache (latest)
    pub best_attestation: Option<Attestation<B>>,

    /// communication (created once, but returned and reused if worker is restarted/reinitialized)
    pub comms: AttestorComms<B>,

    /// runtime api access
    pub backend: Arc<RuntimeApi>,
}

impl<B: BlockT, RA: ProvideRuntimeApi<B>> Worker<B, RA>
where
    B: BlockT,
    RA: ProvideRuntimeApi<B> + Send + Sync + 'static,
    RA::Api: BabeApi<B>,
    RA::Api: BlockBackend<B>,
    <<B as BlockT>::Header as HeaderT>::Number: From<sp_core::U256>,
    H256: From<<B as BlockT>::Hash>,
    <B as BlockT>::Hash: From<H256>,
{
    pub fn new(comms: AttestorComms<B>, backend: Arc<RA>) -> Self {
        Worker {
            best_attestation: None,
            comms,
            backend,
        }
    }

    pub async fn start(mut self) -> Error {
        let mut votes = Box::pin(
            self.comms
                .gossip_engine
                .messages_for(votes_topic::<B>())
                .filter_map(|notification| async move {
                    let message = Message::<B::Hash>::decode(&mut &notification.message[..])
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

    fn verify_vrf(&self, attestation: &Attestation<HashFor<B>>) -> Result<bool, Error> {
        // check if the attestation vrf output is submitted correctly and is eligible for attesting
        let runtime = self.backend.runtime_api();

        let config = runtime.configuration(attestation.vrf_output.block_hash.into())?;
        log::info!(target: "attestor-gossip", "Epoch config: {:?}", config);

        let vrf_epoch = runtime.current_epoch(attestation.vrf_output.block_hash.into())?;
        log::info!(target: "attestor-gossip", "Vrf epoch: {:?}", vrf_epoch);

        let height: NumberFor<B> = U256::from(attestation.header_number).try_into().unwrap();

        let _hash_at_height = runtime
            .block_hash(height)
            .ok()
            .flatten()
            .expect("Genesis block exists; qed");

        let _randomness = vrf_epoch.randomness;

        Ok(true)
    }
}
