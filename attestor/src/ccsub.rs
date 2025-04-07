use anyhow::Result;
use sp_core::H256;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info};

use cc_client::{attestation::CcEvent, AccountId32};

use attestor_primitives::{ChainKey, SignedAttestation};

use crate::engine::Engine;

pub type Randomness = [u8; 32];
pub type RandomnessChange = (u64, Randomness);
pub type AttestationIntervalChange = (ChainKey, u64);

/// Event that can be received from the client
/// - `RandomnessChanged`: Randomness changed
/// - `AttestationIntervalChanged`: Attestation interval changed
pub enum Event {
    RandomnessChanged(RandomnessChange),
    AttestationIntervalChanged(AttestationIntervalChange),
    BlockAttested(SignedAttestation<H256, AccountId32>),
}

pub struct CclientSub {
    engine: Arc<Mutex<Engine>>,
    chain_key: ChainKey,
}

impl CclientSub {
    pub fn new(engine: Arc<Mutex<Engine>>, chain_key: ChainKey) -> Self {
        Self { engine, chain_key }
    }

    pub async fn run(self) -> Result<()> {
        let mut event_sub = self.engine.lock().await.event_sub().await?;

        tokio::spawn(async move {
            loop {
                while let Some(event) = event_sub.next().await {
                    match event {
                        // When randomness changes, re-evaluate the eligibility for the attestor
                        CcEvent::RandomnessChanged((epoch, randomness)) => {
                            info!(
                                "Randomness changed. Epoch: {}, Randomness: {}",
                                epoch,
                                hex::encode(randomness)
                            );

                            let event = Event::RandomnessChanged((epoch, randomness));

                            info!("Locking engine for event");
                            self.engine
                                .lock()
                                .await
                                .note_cc_event(event)
                                .await
                                .expect("Error noting epoch change");
                            info!("Noted epoch change");
                        }
                        CcEvent::AttestationIntervalChanged(ck, interval) => {
                            if self.chain_key != ck {
                                debug!("Ignoring interval change for different chain key");
                                continue;
                            }

                            info!("Attestation interval updated. New interval: {}", interval);

                            let event = Event::AttestationIntervalChanged((ck, interval));

                            self.engine
                                .lock()
                                .await
                                .note_cc_event(event)
                                .await
                                .expect("Error noting interval change");
                        }
                        CcEvent::BlockAttested(attestation) => {
                            if self.chain_key != attestation.chain_key() {
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

                            self.engine
                                .lock()
                                .await
                                .note_cc_event(event)
                                .await
                                .expect("Error noting attestation");
                        }
                        CcEvent::CheckpointReached(checkpoint, ck) => {
                            if self.chain_key != ck {
                                debug!("Ignoring checkpoint for different chain key");
                                continue;
                            }

                            info!(
                                "✅ Checkpoint reached, block: {:}, digest: {:}",
                                checkpoint.block_number, checkpoint.digest
                            );
                        }
                    }
                }
            }
        });

        Ok(())
    }
}
