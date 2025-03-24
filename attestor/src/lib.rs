use anyhow::Result;
use attestation_chain::attestation_fragment::AttestationFragmentSerializable;
use eth::Client;
use sp_core::H256;
use std::collections::BTreeSet;
use tracing::{error, info, warn};

pub mod attestation;
pub mod cc3;
pub mod engine;
pub mod eth_sub;
pub mod fragment;
pub mod merkle;
mod retry;

use attestor_primitives::{Attestation as AttestationPrimitive, AttestorId};
use cc3::Error;
use cc_client::attestation::CcEvent;
use creditcoin3_attestor_gossip::communication::Attestation;

#[derive(Debug, Clone)]
/// Attestor server is configured using `Config`
pub struct Server {
    config: Config,
    // Keeps track of which attestations have been voted for
    voted_for: BTreeSet<u64>,
}

#[derive(Debug, Clone)]
/// Server configuration
/// - `eth_rpc_url`: Source chain RPC url
/// - `eth_start_block`: Start block for the source chain
/// - `cc3_rpc_url`: Creditcoin RPC url (must have rpc + websocket features)
/// - `cc3_key`: Mnemonic for a creditcoin3 account
pub struct Config {
    pub eth_rpc_url: String,
    pub cc3_rpc_url: String,
    pub cc3_key: String,
    //pub bls_key: [u8; 32],
}

impl Server {
    /// Create a new server based on `Config`
    #[must_use]
    pub fn new(config: Config) -> Self {
        Server {
            config,
            voted_for: BTreeSet::new(),
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        // Construct the attestation engine
        let mut engine = engine::Engine::new(&self.config).await?;

        self.respond_to_new_events_and_attestations(&mut engine)
            .await
    }

    async fn respond_to_new_events_and_attestations(
        &mut self,
        engine: &mut engine::Engine,
    ) -> Result<()> {
        let chain_key = engine.chain_key();

        // Start the attestation engine
        engine.evaluate(None).await?;

        let mut event_sub = engine.event_sub().await?;

        // Biased tokio select, we will prioritze listening to randomness changed events
        // because this will re-evaluate the eligibility for attestors
        loop {
            tokio::select! {
                biased;
                // Enact on new events from the chain
                // These have influence on the attestation engine
                maybe_event = event_sub.next() => {
                    let Some(event) = maybe_event else { break; };

                    match event {
                        // When randomness changes, re-evaluate the eligibility for the attestor
                        CcEvent::RandomnessChanged((epoch, randomness)) => {
                            info!("Randomness changed. Epoch: {}, Randomness: {}", epoch, hex::encode(randomness));
                            let start_block = self.voted_for.last().copied();
                            engine.evaluate(start_block).await?;
                        }
                        CcEvent::AttestationIntervalChanged(_, new_interval) => {
                            info!(
                                "Attestation interval updated. New interval: {}", new_interval
                            );
                            let need_restart = engine.change_interval(new_interval);

                            if need_restart {
                                let start_block = self.voted_for.last().copied();
                                engine.restart(start_block).await?;
                            }

                        },
                        CcEvent::CheckpointReached(checkpoint, ck) => {
                            info!("✅ Checkpoint reached, block: {:}, digest: {:}", checkpoint.block_number, checkpoint.digest);
                            if chain_key != ck {
                                return Ok(());
                            }
                            let start_block = self.voted_for.last().copied();
                            engine.evaluate(start_block).await?;
                        }
                        CcEvent::BlockAttested(_) => ()
                    }
                },
                // Poll the attestation engine for the next attestation
                maybe_attestation = engine.next() => {
                    let Some(attestation_to_submit) = maybe_attestation else { break; };

                    let round = attestation_to_submit.round();
                    match self.handle_attestation(chain_key, engine.eth_client(), engine.cc_client(), attestation_to_submit).await {
                        Ok(attestation) => {
                            let digest = attestation.digest();
                            info!("Going to submit attestation with digest: {:?} for round: {:?} ...",digest, round);

                            // Submit the attestation to the chain
                            if engine.submit_attestation(attestation.clone()).await.is_err() {
                                error!("Failed to submit attestation with digest: {:?}, round: {:?}", digest, round);
                                // We can clear voted for here because the engine will restart at the last finalized attestation
                                // Possible equivocation scenario here
                                self.voted_for.clear();
                            } else {
                                self.voted_for.insert(round.1);
                                info!("📝 Attestation with digest: {:?} for round: {:?} submitted!", digest, round);
                            }
                        },
                        Err(e) => {
                            if e.is_not_selected_error() {
                                warn!("Failed to create proof of inclusion, attestor not selected.");
                            } else if e.is_duplicate_submission() {
                                warn!("Attestation for round: {:?} already submitted, skipping", round);
                            } else {
                                error!("Non-retryable error submitting attestation: {:?}, round: {:?}", e, round);
                                return Err(e.into());
                            }
                        }
                    };
                    info!("Waiting for next attestation to come in...");
                },
                // Do nothing if the engine is not ready
                else => {
                    break;
                }
            }
        }

        Ok(())
    }

    /// Check if we can sign the attestation, if we cannot sign it it means we are not selected
    /// In the case we are selected we need to build a fragment for the interval
    async fn handle_attestation(
        &mut self,
        chain_key: u64,
        eth_client: &Client,
        cc3_client: &cc3::Client,
        attestation: AttestationPrimitive<H256>,
    ) -> Result<Attestation<H256, AttestorId>, Error> {
        let mut signed_attestation = cc3_client.sign_attestation(attestation).await?;

        let header_number = signed_attestation.attestation_data.header_number;
        info!("Attestor selected for block({})", header_number);

        // Exit early if the attestation has already been submitted
        if self.voted_for.contains(&header_number) {
            return Err(Error::DuplicateSubmission);
        }

        let last_attestation = cc3_client.get_last_attestation(chain_key).await?;

        // The start block is the last block we voted for
        // or the last block that was attested on chain
        // or genesis if we have never voted
        let start_block = if let Some(header_number) = self.voted_for.last().copied() {
            header_number
        } else if let Some(attestation) = last_attestation {
            attestation.attestation.header_number
        } else {
            0
        };

        // Create the fragment for the signed attestation
        // This is the continuity proof of this signed attestation
        let fragment: AttestationFragmentSerializable = retry::ret(
            || async {
                let mut signed_attestation = signed_attestation.clone();
                fragment::create(chain_key, start_block, &mut signed_attestation, eth_client).await
            },
            10,
            10,
            None,
        )
        .await?;

        // Set the continuity proof
        signed_attestation.continuity_proof = fragment;

        Ok(signed_attestation)
    }
}
