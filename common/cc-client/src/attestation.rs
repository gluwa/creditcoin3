use anyhow::Result;
use sp_core::H256;
use subxt::error::Error as SubxtError;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, info};

pub use subxt::utils::AccountId32;

use attestor_primitives::{ChainId, SignedAttestation};

use crate::cc3::{attestation::events::BlockAttested, randomness::events::StoreRandomnessForEpoch};

use crate::{Client, Randomness};

pub enum CcEvent {
    BlockAttestedEvent(SignedAttestation<H256, AccountId32>),
    RandomnessChangedEvent((u64, Randomness)),
}

const BUFFER_SIZE: usize = 100;

/// `Subscription` is a struct that references to a receiving end of a channel where claims are pushed upon
/// It has a handle to cancel the subscription
#[derive(Debug)]
pub struct Subscription {
    receiver: mpsc::Receiver<CcEvent>,
    handle: JoinHandle<Result<(), Error>>,
}

impl Subscription {
    /// Cancel the subscription
    pub fn cancel(&self) -> Result<()> {
        // Cancel the subscription task
        debug!("Canceling subscription");
        self.handle.abort();
        Ok(())
    }

    /// Get the next proof
    pub async fn next(&mut self) -> Option<CcEvent> {
        // Receive the next proof from the channel
        self.receiver.recv().await
    }
}

// See pallets/attestation-poc/lib.rs
const ATTESTATION_MODULE: &str = "Attestation";
const ATTESTATION_SUBMITTED_EVENT: &str = "BlockAttested";

// See pallet/randomness/lib.rs
const RANDOMNESS_MODULE: &str = "Randomness";
const RANDOMNESS_CHANGED_EVENT: &str = "StoreRandomnessForEpoch";

#[derive(Error, Debug)]
pub enum Error {
    #[error("subxt error {0}")]
    SubxtError(#[from] SubxtError),
    #[error("client error {0}")]
    ClientError(#[from] anyhow::Error),
}

impl Client {
    pub fn subscribe_events(&self, filter: ChainId) -> Result<Subscription, Error> {
        // Create the channel with buffer size
        let (sender, receiver) = mpsc::channel(BUFFER_SIZE);

        // Clone the api and send it on the tokio task
        let api = self.api.clone();

        let handle = tokio::spawn(async move {
            let mut blocks_sub = api.blocks().subscribe_finalized().await?;
            info!("Subscription started, streaming finalized blocks...");

            while let Some(block) = blocks_sub.next().await {
                let block = block?;

                let block_number = block.header().number;
                debug!("Got block #{block_number}:");
                let block_hash = block.hash();
                debug!("Block Hash: {block_hash}");

                let events = block.events().await?;

                for event in events.iter() {
                    let event = event.unwrap();
                    debug!(
                        "event pallet: {}, event variant: {}",
                        event.pallet_name(),
                        event.variant_name()
                    );

                    match (event.pallet_name(), event.variant_name()) {
                        (ATTESTATION_MODULE, ATTESTATION_SUBMITTED_EVENT) => {
                            if let Ok(Some(evt)) = event.as_event::<BlockAttested>() {
                                debug!("attestation chain_id: {:?}", evt.0);

                                // If the filter is not empty, check if the chain_id is in the filter
                                if filter != evt.0 {
                                    continue;
                                }

                                let attestation: SignedAttestation<H256, AccountId32> =
                                    evt.1.into();

                                debug!("attestation digest: {:?}", attestation.digest());

                                if sender
                                    .send(CcEvent::BlockAttestedEvent(attestation))
                                    .await
                                    .is_err()
                                {
                                    // The receiver has been dropped
                                    break;
                                }
                            }
                        }
                        (RANDOMNESS_MODULE, RANDOMNESS_CHANGED_EVENT) => {
                            if let Ok(Some(evt)) = event.as_event::<StoreRandomnessForEpoch>() {
                                debug!(
                                    "randomness epoch (index: {}, randomnes: {:?})",
                                    evt.epoch_index, evt.randomness
                                );

                                if sender
                                    .send(CcEvent::RandomnessChangedEvent((
                                        evt.epoch_index,
                                        evt.randomness,
                                    )))
                                    .await
                                    .is_err()
                                {
                                    debug!("The receiver has been dropped");
                                    // The receiver has been dropped
                                    break;
                                }
                            }
                        }
                        (_m, _e) => (),
                    }
                }
            }

            Ok(())
        });

        Ok(Subscription { receiver, handle })
    }
}
