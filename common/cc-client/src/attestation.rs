use anyhow::Result;
use sp_core::H256;
use subxt::{error::Error as SubxtError, OnlineClient, SubstrateConfig};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, info};

pub use subxt::utils::AccountId32;

use attestor_primitives::{ChainId, SignedAttestation};

use crate::cc3::attestation::events::BlockAttested;
use crate::Client;

const BUFFER_SIZE: usize = 100;

/// `AttestationSubscription` is a struct that references to a receiving end of a channel where claims are pushed upon
/// It has a handle to cancel the subscription
#[derive(Debug)]
pub struct AttestationSubscription {
    receiver: mpsc::Receiver<SignedAttestation<H256, AccountId32>>,
    handle: JoinHandle<Result<(), Error>>,
}

impl AttestationSubscription {
    /// Cancel the subscription
    pub fn cancel(&self) -> Result<()> {
        // Cancel the subscription task
        debug!("Canceling subscription");
        self.handle.abort();
        Ok(())
    }

    /// Get the next proof
    pub async fn next(&mut self) -> Option<SignedAttestation<H256, AccountId32>> {
        // Receive the next proof from the channel
        self.receiver.recv().await
    }
}

// See pallets/attestation-poc/lib.rs
const ATTESTATION_MODULE: &str = "Attestation";
const ATTESTATION_SUBMITTED_EVENT: &str = "BlockAttested";
const ATTESTATION_SUBMISSION_EXT_INDEX: u8 = 9;

#[derive(Error, Debug)]
pub enum Error {
    #[error("subxt error {0}")]
    SubxtError(#[from] SubxtError),
    #[error("client error {0}")]
    ClientError(#[from] anyhow::Error),
}

impl Client {
    pub async fn subscribe_attestations_submissions(
        &self,
        filter: Vec<ChainId>,
    ) -> Result<AttestationSubscription, Error> {
        let api = self.get_substrate_client().await?;

        // Create the channel with buffer size
        let (sender, receiver) = mpsc::channel(BUFFER_SIZE);

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
                for extrinsic in extrinsics.iter() {
                    let ext = extrinsic?;

                    match (ext.pallet_name()?, ext.variant_index()) {
                        (ATTESTATION_MODULE, ATTESTATION_SUBMISSION_EXT_INDEX) => {
                            let events = ext.events().await?;

                            for event in events.iter() {
                                if event.is_err() {
                                    continue;
                                };

                                let event = event?;

                                match (event.pallet_name(), event.variant_name()) {
                                    (ATTESTATION_MODULE, ATTESTATION_SUBMITTED_EVENT) => {
                                        if let Ok(Some(evt)) = event.as_event::<BlockAttested>() {
                                            debug!("attestation chain_id: {:?}", evt.0);

                                            // If the filter is not empty, check if the chain_id is in the filter
                                            if filter.is_empty() && !filter.contains(&evt.0) {
                                                continue;
                                            }

                                            let attestation: SignedAttestation<H256, AccountId32> =
                                                evt.1.into();

                                            debug!(
                                                "attestation digest: {:?}",
                                                attestation.digest()
                                            );

                                            if sender.send(attestation).await.is_err() {
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

        Ok(AttestationSubscription { receiver, handle })
    }
}
