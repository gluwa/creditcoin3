use anyhow::Result;
use sp_core::H256;
use tracing::{debug, info};

use cc_client::{attestation::CcEvent, AccountId32};

use attestor_primitives::{AttestationCheckpoint, ChainKey, SignedAttestation};

use crate::engine::AsyncEngine;

pub type Randomness = [u8; 32];
pub type RandomnessChange = (u64, Randomness);
pub type AttestationIntervalChange = (ChainKey, u64);

/// Event that can be received from the client
/// - `RandomnessChanged`: Randomness changed
/// - `AttestationIntervalChanged`: Attestation interval changed
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    RandomnessChanged(RandomnessChange),
    AttestationIntervalChanged(AttestationIntervalChange),
    BlockAttested(SignedAttestation<H256, AccountId32>),
    CheckpointReached(ChainKey, AttestationCheckpoint),
}

pub struct CclientSub {
    engine: AsyncEngine,
}

impl CclientSub {
    pub fn new(engine: AsyncEngine) -> Self {
        Self { engine }
    }

    pub async fn run(self) -> Result<()> {
        let mut engine = self.engine.clone();
        let mut event_sub = engine.event_sub().await?;

        while let Some(event) = event_sub.next().await {
            match event {
                CcEvent::RandomnessChanged((epoch, randomness)) => {
                    info!(
                        "🕒 Epoch rotated. Epoch: {}, Randomness: {}",
                        epoch,
                        hex::encode(randomness)
                    );

                    let event = Event::RandomnessChanged((epoch, randomness));
                    debug!("Locking engine for event");
                    engine.note_cc_event(event).await?;
                }

                CcEvent::AttestationIntervalChanged(ck, interval) => {
                    if engine.chain_key != ck {
                        debug!("Ignoring interval change for different chain key");
                        continue;
                    }

                    info!(
                        "📢 Attestation interval updated. New interval: {}",
                        interval
                    );
                    let event = Event::AttestationIntervalChanged((ck, interval));
                    engine.note_cc_event(event).await?;
                }

                CcEvent::BlockAttested(attestation) => {
                    if engine.chain_key != attestation.chain_key() {
                        debug!("Ignoring attestation for different chain key");
                        continue;
                    }

                    let last_attested_header = attestation.header_number();
                    info!(
                        "📝 Block({}) attested for, digest: {:?}",
                        last_attested_header,
                        attestation.digest()
                    );

                    let event = Event::BlockAttested(attestation);
                    engine.note_cc_event(event).await?;
                }

                CcEvent::CheckpointReached(ck, checkpoint) => {
                    if engine.chain_key != ck {
                        debug!("Ignoring checkpoint for different chain key");
                        continue;
                    }

                    info!(
                        "✅ Checkpoint reached, block: {:}, digest: {:}",
                        checkpoint.block_number, checkpoint.digest
                    );

                    engine
                        .note_cc_event(Event::CheckpointReached(ck, checkpoint))
                        .await?;
                }
            };
        }

        Err(anyhow::Error::msg(
            "Creditcoin subscription stopped, no more events",
        ))
    }
}
