use anyhow::Result;
use subxt::utils::AccountId32;
use subxt::{OnlineClient, SubstrateConfig};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::debug;

use crate::cc3::prover::calls::types::submit_claim::Claim;
use crate::cc3::prover::events::ProverClaimSubmitted;

use crate::Client;

pub type Proof = Vec<Claim>;

const BUFFER_SIZE: usize = 100;

/// ClaimSubscription is a struct that references to a receiving end of a channel where claims are pushed upon
/// It has a handle to cancel the subscription
pub struct ClaimSubscription {
    receiver: mpsc::Receiver<Claim>,
    handle: JoinHandle<()>,
}

impl ClaimSubscription {
    /// Cancel the subscription
    pub async fn cancel(self) -> Result<()> {
        // Cancel the subscription task
        self.handle.abort();
        Ok(())
    }

    /// Get the next proof
    pub async fn next(&mut self) -> Option<Claim> {
        // Receive the next proof from the channel
        self.receiver.recv().await
    }
}

impl Client {
    pub async fn subscribe_claim_submission_events(&self) -> Result<ClaimSubscription> {
        let api = self.get_substrate_client().await?;

        // Create the channel with buffer size
        let (sender, receiver) = mpsc::channel(BUFFER_SIZE);

        let prover_account_id = AccountId32(self.keypair.public_key().0);

        // Clone the api and send it on the tokio task
        let api = api.clone();

        let handle = tokio::spawn(async move {
            let mut blocks_sub = api.blocks().subscribe_finalized().await.unwrap();

            while let Some(block) = blocks_sub.next().await {
                let block = block.unwrap();

                let block_number = block.header().number;
                debug!("Got block #{block_number}:");
                let block_hash = block.hash();
                debug!("Block Hash: {block_hash}");

                // Filter on a Claim Submission event
                // If we found one, push it on the channel for the client to handle
                let extrinsics: subxt::blocks::Extrinsics<
                    SubstrateConfig,
                    OnlineClient<SubstrateConfig>,
                > = block.extrinsics().await.unwrap();
                for ext in extrinsics.iter() {
                    let ext = ext.unwrap();

                    match (ext.pallet_name().unwrap(), ext.variant_index()) {
                        ("ProverModule", 4) => {
                            let events = ext.events().await.unwrap();

                            for evt in events.iter() {
                                if evt.is_err() {
                                    continue;
                                };

                                let evt = evt.unwrap();

                                match (evt.pallet_name(), evt.variant_name()) {
                                    ("ProverModule", "ProverClaimSubmitted") => {
                                        if let Ok(Some(evt)) =
                                            evt.as_event::<ProverClaimSubmitted>()
                                        {
                                            debug!("claim source: {:?}", evt.1);
                                            debug!("claim target prover: {:?}", evt.1);
                                            debug!("claim hash: {:?}", evt.2);

                                            if evt.1 != prover_account_id {
                                                continue;
                                            };

                                            if sender.send(evt.3).await.is_err() {
                                                // The receiver has been dropped
                                                break;
                                            }
                                        }
                                    }
                                    (_m, _e) => (),
                                }
                            }
                        }
                        (_m, _i) => (),
                    };
                }
            }
        });

        Ok(ClaimSubscription { receiver, handle })
    }
}
