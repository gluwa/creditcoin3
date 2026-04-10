use std::sync::Arc;

use anyhow::Result;
use subxt::{error::Error as SubxtError, events::StaticEvent};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

pub use subxt::utils::AccountId32;

use attestor_primitives::{AttestationCheckpoint, ChainKey, Digest};

use crate::cc3::{
    attestation::events::{
        AttestationChainGenesisBlockNumberSet, AttestationIntervalChanged, AttestorActivated,
        AttestorChilled, AttestorsElected, BlockAttested, CheckpointIntervalChanged,
        CheckpointReached, RevertedAttestationChainTo, TargetSampleSizeChanged,
    },
    randomness::events::StoreRandomnessForEpoch,
    staking::events::Kicked,
};

use crate::{Client, Randomness};

#[derive(Debug, Clone)]
pub struct BlockAttestedMetadata {
    pub chain_key: ChainKey,
    pub header_number: u64,
    pub digest: Digest,
}

impl BlockAttestedMetadata {
    #[must_use]
    pub fn chain_key(&self) -> ChainKey {
        self.chain_key
    }

    #[must_use]
    pub fn header_number(&self) -> u64 {
        self.header_number
    }

    #[must_use]
    pub fn digest(&self) -> Digest {
        self.digest
    }
}

#[derive(Debug, Clone)]
pub enum CcEvent {
    BlockAttested(BlockAttestedMetadata),
    RandomnessChanged((u64, Randomness)),
    /// Source chain key is included so multi-chain subscribers can route events.
    CheckpointReached(ChainKey, AttestationCheckpoint),
    AttestationIntervalChanged(ChainKey, u64),
    TargetSampleSizeChanged(ChainKey, u32),
    CheckpointIntervalChanged(ChainKey, u64),
    AttestorsElected(ChainKey, Vec<AccountId32>),
    AttestorActivated(ChainKey, AccountId32),
    AttestorChilled(ChainKey, AccountId32),
    /// Staking pallet `Kicked` (nominator/stash); not scoped to a source chain — no `ChainKey` on-chain.
    AttestorKicked(AccountId32),
    AttestationChainGenesisBlockNumberSet(ChainKey, u64),
    RevertedAttestationChainTo(ChainKey, u64, Digest),
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

    /// Get the next creditcoin event from the subscription
    pub async fn next(&mut self) -> Option<CcEvent> {
        // Receive the next proof from the channel
        self.receiver.recv().await
    }
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("subxt error {0}")]
    SubxtError(#[from] SubxtError),
    #[error("client error {0}")]
    ClientError(#[from] anyhow::Error),
    #[error("error {0}")]
    Error(#[from] crate::Error),
    #[error("Subscription connection lost: {0}")]
    SubscriptionConnectionLost(SubxtError),
}

impl Client {
    /// Subscribe to CC3 events for a single source chain.
    #[allow(clippy::too_many_lines)]
    #[allow(clippy::result_large_err)]
    pub fn subscribe_events(&self, filter: ChainKey) -> Result<Subscription, Error> {
        self.subscribe_events_chains(std::slice::from_ref(&filter))
    }

    /// Subscribe to CC3 events for one or more source chains (single finalized block stream).
    ///
    /// `chain_keys` is copied once into the subscription task; filtering uses `[ChainKey]::contains`
    /// (few keys, no `HashSet` / `Arc`).
    #[allow(clippy::too_many_lines)]
    #[allow(clippy::result_large_err)]
    pub fn subscribe_events_chains(&self, chain_keys: &[ChainKey]) -> Result<Subscription, Error> {
        if chain_keys.is_empty() {
            return Err(anyhow::anyhow!(
                "subscribe_events_chains: at least one chain key required"
            )
            .into());
        }

        // Create the channel with buffer size
        let (sender, receiver) = mpsc::channel(BUFFER_SIZE);

        // Wrap filter keys in Arc for shared immutable access in the spawned task.
        let api = self.api().clone();
        let chain_filter: Arc<[ChainKey]> = chain_keys.into();

        let handle = tokio::spawn(async move {
            let mut blocks_sub = api.blocks().subscribe_finalized().await?;
            info!("Subscription started, streaming finalized blocks...");

            loop {
                match blocks_sub.next().await {
                    Some(Ok(block)) => {
                        let events = block.events().await?;
                        for event in Self::extract_events(&chain_filter, &events) {
                            // FIXME: remove this `unwrap`
                            if sender.send(event.unwrap()).await.is_err() {
                                break;
                            }
                        }
                    }
                    Some(Err(e)) => {
                        error!(
                            "Subscription error while fetching next block: {:?}. Panicking!",
                            e
                        );
                        return Err(Error::SubscriptionConnectionLost(e));
                    }
                    None => {
                        info!("Block subscription stream closed by the backend.");
                        break;
                    }
                }
            }
            info!("Subscription task finished gracefully.");
            Ok(())
        });

        Ok(Subscription { receiver, handle })
    }

    #[tracing::instrument(skip(events))]
    #[allow(clippy::too_many_lines)]
    pub fn extract_events<'a>(
        chain_filter: &'a [ChainKey],
        events: &'a subxt::events::Events<subxt::SubstrateConfig>,
    ) -> impl Iterator<Item = Result<CcEvent, subxt::Error>> + 'a {
        events.iter().filter_map(move |event| match event {
            Ok(event) => {
                let span = tracing::debug_span!(
                    "event",
                    pallet = event.pallet_name(),
                    variant = event.variant_name()
                );
                let _enter = span.enter();

                match (event.pallet_name(), event.variant_name()) {
                    (BlockAttested::PALLET, BlockAttested::EVENT) => {
                        let Ok(Some(event)) = event.as_event::<BlockAttested>() else {
                            tracing::error!("Invalid event mapping");
                            return None;
                        };

                        let BlockAttested(chain_key, header_number, digest) = event;

                        if !chain_filter.contains(&chain_key) {
                            return None;
                        }

                        Some(Ok(CcEvent::BlockAttested(BlockAttestedMetadata {
                            chain_key,
                            header_number,
                            digest: Digest::from(digest.0),
                        })))
                    }
                    (StoreRandomnessForEpoch::PALLET, StoreRandomnessForEpoch::EVENT) => {
                        let Ok(Some(event)) = event.as_event::<StoreRandomnessForEpoch>() else {
                            tracing::error!("Invalid event mapping");
                            return None;
                        };

                        Some(Ok(CcEvent::RandomnessChanged((
                            event.epoch_index,
                            event.randomness,
                        ))))
                    }
                    (CheckpointReached::PALLET, CheckpointReached::EVENT) => {
                        let Ok(Some(event)) = event.as_event::<CheckpointReached>() else {
                            tracing::error!("Invalid event mapping");
                            return None;
                        };

                        let CheckpointReached(chain_key, checkpoint) = event;

                        if !chain_filter.contains(&chain_key) {
                            return None;
                        }

                        Some(Ok(CcEvent::CheckpointReached(chain_key, checkpoint.into())))
                    }
                    (TargetSampleSizeChanged::PALLET, TargetSampleSizeChanged::EVENT) => {
                        let Ok(Some(event)) = event.as_event::<TargetSampleSizeChanged>() else {
                            tracing::error!("Invalid event mapping");
                            return None;
                        };

                        let TargetSampleSizeChanged(chain_key, new_sample_size) = event;

                        if !chain_filter.contains(&chain_key) {
                            return None;
                        }

                        Some(Ok(CcEvent::TargetSampleSizeChanged(
                            chain_key,
                            new_sample_size,
                        )))
                    }
                    (AttestationIntervalChanged::PALLET, AttestationIntervalChanged::EVENT) => {
                        let Ok(Some(event)) = event.as_event::<AttestationIntervalChanged>() else {
                            tracing::error!("Invalid event mapping");
                            return None;
                        };

                        let AttestationIntervalChanged(chain_key, interval_new) = event;

                        if !chain_filter.contains(&chain_key) {
                            return None;
                        }

                        Some(Ok(CcEvent::AttestationIntervalChanged(
                            chain_key,
                            interval_new,
                        )))
                    }
                    (CheckpointIntervalChanged::PALLET, CheckpointIntervalChanged::EVENT) => {
                        let Ok(Some(event)) = event.as_event::<CheckpointIntervalChanged>() else {
                            tracing::error!("Invalid event mapping");
                            return None;
                        };

                        let CheckpointIntervalChanged(chain_key, interval_new) = event;

                        if !chain_filter.contains(&chain_key) {
                            return None;
                        }

                        Some(Ok(CcEvent::CheckpointIntervalChanged(
                            chain_key,
                            u64::from(interval_new),
                        )))
                    }
                    (AttestorsElected::PALLET, AttestorsElected::EVENT) => {
                        let Ok(Some(event)) = event.as_event::<AttestorsElected>() else {
                            tracing::error!("Invalid event mapping");
                            return None;
                        };

                        let AttestorsElected {
                            epoch: _,
                            chain_key,
                            attestors,
                        } = event;

                        if !chain_filter.contains(&chain_key) {
                            return None;
                        }

                        Some(Ok(CcEvent::AttestorsElected(chain_key, attestors)))
                    }
                    (AttestorActivated::PALLET, AttestorActivated::EVENT) => {
                        let Ok(Some(event)) = event.as_event::<AttestorActivated>() else {
                            tracing::error!("Invalid event mapping");
                            return None;
                        };

                        let AttestorActivated(chain_key, account_id, _bls_public_key) = event;

                        if !chain_filter.contains(&chain_key) {
                            return None;
                        }

                        Some(Ok(CcEvent::AttestorActivated(chain_key, account_id)))
                    }
                    (Kicked::PALLET, Kicked::EVENT) => {
                        let Ok(Some(event)) = event.as_event::<Kicked>() else {
                            tracing::error!("Invalid event mapping");
                            return None;
                        };

                        let Kicked {
                            nominator: _,
                            stash,
                        } = event;

                        Some(Ok(CcEvent::AttestorKicked(stash)))
                    }
                    (AttestorChilled::PALLET, AttestorChilled::EVENT) => {
                        let Ok(Some(event)) = event.as_event::<AttestorChilled>() else {
                            tracing::error!("Invalid event mapping");
                            return None;
                        };

                        let AttestorChilled(chain_key, account_id) = event;

                        if !chain_filter.contains(&chain_key) {
                            return None;
                        }

                        Some(Ok(CcEvent::AttestorChilled(chain_key, account_id)))
                    }
                    (
                        AttestationChainGenesisBlockNumberSet::PALLET,
                        AttestationChainGenesisBlockNumberSet::EVENT,
                    ) => {
                        let Ok(Some(event)) =
                            event.as_event::<AttestationChainGenesisBlockNumberSet>()
                        else {
                            tracing::error!("Invalid event mapping");
                            return None;
                        };

                        let AttestationChainGenesisBlockNumberSet(chain_key, block_number) = event;

                        if !chain_filter.contains(&chain_key) {
                            return None;
                        }

                        Some(Ok(CcEvent::AttestationChainGenesisBlockNumberSet(
                            chain_key,
                            block_number,
                        )))
                    }
                    (RevertedAttestationChainTo::PALLET, RevertedAttestationChainTo::EVENT) => {
                        let Ok(Some(event)) = event.as_event::<RevertedAttestationChainTo>() else {
                            tracing::error!("Invalid event mapping");
                            return None;
                        };

                        let RevertedAttestationChainTo {
                            chain_key,
                            checkpoint_height,
                            checkpoint_digest,
                        } = event;

                        if !chain_filter.contains(&chain_key) {
                            return None;
                        }

                        Some(Ok(CcEvent::RevertedAttestationChainTo(
                            chain_key,
                            checkpoint_height,
                            Digest::from(checkpoint_digest.0),
                        )))
                    }
                    (_module, _event) => None,
                }
            }
            Err(e) => Some(Err(e)),
        })
    }
}
