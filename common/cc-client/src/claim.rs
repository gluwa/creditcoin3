use anyhow::Result;
use sp_core::H256;
use subxt::utils::AccountId32;
use subxt::{error::Error as SubxtError, OnlineClient, SubstrateConfig};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, info};

pub use crate::cc3::prover::calls::types::submit_claim::Claim as Cc3Claim;
use crate::cc3::prover::events::ProverClaimSubmitted;

use crate::Client;

#[derive(Debug)]
pub struct Claim {
    pub source: AccountId32,
    pub target: AccountId32,
    pub claim: Cc3Claim,
    pub hash: H256,
}

const BUFFER_SIZE: usize = 100;

/// ClaimSubscription is a struct that references to a receiving end of a channel where claims are pushed upon
/// It has a handle to cancel the subscription
#[derive(Debug)]
pub struct ClaimSubscription {
    receiver: mpsc::Receiver<Claim>,
    handle: JoinHandle<Result<(), Error>>,
}

impl ClaimSubscription {
    /// Cancel the subscription
    pub async fn cancel(&self) -> Result<()> {
        // Cancel the subscription task
        debug!("Canceling subscription");
        self.handle.abort();
        Ok(())
    }

    /// Get the next proof
    pub async fn next(&mut self) -> Option<Claim> {
        // Receive the next proof from the channel
        self.receiver.recv().await
    }
}

// See pallets/prover/lib.rs
const PROVER_MODULE: &str = "Prover";
const CLAIM_SUBMITTED_EVENT: &str = "ProverClaimSubmitted";
const CLAIM_SUBMISSION_EXT_INDEX: u8 = 3;

#[derive(Error, Debug)]
pub enum Error {
    #[error("subxt error {0}")]
    SubxtError(#[from] SubxtError),
    #[error("client error {0}")]
    ClientError(#[from] anyhow::Error),
}

impl Client {
    pub async fn subscribe_claim_submission_events(&self) -> Result<ClaimSubscription, Error> {
        let api = self.get_substrate_client().await?;

        // Create the channel with buffer size
        let (sender, receiver) = mpsc::channel(BUFFER_SIZE);

        let account_id = AccountId32(self.keypair.public_key().0);

        // Clone the api and send it on the tokio task
        let api = api.clone();

        let handle = tokio::spawn(async move {
            let mut blocks_sub = api.blocks().subscribe_finalized().await?;
            info!("Subscription started, streaming finalized blocks...");

            while let Some(block) = blocks_sub.next().await {
                let block = block?;

                let block_number = block.header().number;
                debug!("Got block #{block_number}:");
                let block_hash = block.hash();
                debug!("Block Hash: {block_hash}");

                // Filter on a Claim Submission event
                // If we found one, push it on the channel for the client to handle
                let extrinsics: subxt::blocks::Extrinsics<
                    SubstrateConfig,
                    OnlineClient<SubstrateConfig>,
                > = block.extrinsics().await?;
                for ext in extrinsics.iter() {
                    let ext = ext?;

                    match (ext.pallet_name()?, ext.variant_index()) {
                        (PROVER_MODULE, CLAIM_SUBMISSION_EXT_INDEX) => {
                            let events = ext.events().await?;

                            for evt in events.iter() {
                                if evt.is_err() {
                                    continue;
                                };

                                let evt = evt?;

                                match (evt.pallet_name(), evt.variant_name()) {
                                    (PROVER_MODULE, CLAIM_SUBMITTED_EVENT) => {
                                        if let Ok(Some(evt)) =
                                            evt.as_event::<ProverClaimSubmitted>()
                                        {
                                            debug!("claim source: {:?}", evt.1);
                                            debug!("claim target prover: {:?}", evt.1);
                                            debug!("claim hash: {:?}", evt.2);

                                            if evt.1 != account_id {
                                                continue;
                                            };

                                            if sender
                                                .send(Claim {
                                                    source: evt.0,
                                                    target: evt.1,
                                                    hash: evt.2,
                                                    claim: evt.3,
                                                })
                                                .await
                                                .is_err()
                                            {
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

            Ok(())
        });

        Ok(ClaimSubscription { receiver, handle })
    }
}
