use anyhow::Result;
use eth::Client;
use sp_core::H256;
use std::{thread, time::Duration};
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
    // Last sent attestation by the attestor
    last_sent_attestation: Option<Attestation<H256, AttestorId>>,
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
            last_sent_attestation: None,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        // Construct the attestation engine
        let mut engine = engine::Engine::new(&self.config).await?;

        self.respond_to_new_events_and_attestations(&mut engine)
            .await?;

        Ok(())
    }

    async fn respond_to_new_events_and_attestations(
        &mut self,
        engine: &mut engine::Engine,
    ) -> Result<()> {
        let chain_key = engine.chain_key();

        // Start the attestation engine
        engine.evaluate(None).await?;

        let mut event_sub = engine.event_sub()?;

        // Biased tokio select, we will prioritze listening to randomness changed events
        // because this will re-evaluate the eligibility for attestors
        loop {
            tokio::select! {
                biased;
                // Enact on new events from the chain
                // These have influence on the attestation engine
                Some(event) = event_sub.next() => {
                    match event {
                        // When randomness changes, re-evaluate the eligibility for the attestor
                        CcEvent::RandomnessChanged((epoch, randomness)) => {
                            info!("Randomness changed. Epoch: {}, Randomness: {}", epoch, hex::encode(randomness));
                            let start_block = self.last_sent_attestation.as_ref().map(|a| a.attestation_data.header_number);
                            engine.evaluate(start_block).await?;
                        }
                        CcEvent::AttestationIntervalChanged(_, new_interval) => {
                            info!(
                                "Attestation interval updated. New interval: {}", new_interval
                            );
                            engine.change_interval(new_interval);
                        },
                        CcEvent::CheckpointReached(checkpoint, ck) => {
                            info!("✅ Checkpoint reached: {:?}", checkpoint);
                            if chain_key != ck {
                                return Ok(());
                            }
                            let start_block = self.last_sent_attestation.as_ref().map(|a| a.attestation_data.header_number);
                            engine.evaluate(start_block).await?;
                        }
                        CcEvent::BlockAttested(_) => ()
                    }
                },
                // Poll the attestation engine for the next attestation
                Some(attestation_to_submit) = engine.next() => {
                    let round = attestation_to_submit.round();
                    match self.handle_attestation(chain_key, engine.eth_client(), engine.cc_client(), attestation_to_submit).await {
                        Ok(attestation) => {
                            let digest = attestation.digest();
                            info!("Submitting attestation with digest: {:?}, round: {:?}",digest, round);
                            // Submit the attestation to the chain
                            engine.cc_client().submit_attestation::<H256>(attestation.clone()).await?;
                            self.last_sent_attestation = Some(attestation);
                            info!("Attestation with digest: {:?} submitted, round: {:?}", digest, round);
                            // Sleep for a while to avoid spamming the chain
                            thread::sleep(Duration::from_secs(6));
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
            }
        }
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
        let signed_attestation = cc3_client.sign_attestation(attestation).await?;

        let header_number = signed_attestation.attestation_data.header_number;
        info!("Attestor selected for block({})", header_number);

        // Exit early if the attestation has already been submitted
        if let Some(last_sent_attestation) = self.last_sent_attestation.clone() {
            if last_sent_attestation.attestation_data.header_number >= header_number {
                warn!("Attestation already submitted, skipping");
                return Err(Error::DuplicateSubmission);
            }
        }

        let last_attestation = cc3_client.get_last_attestation(chain_key).await?;

        let start_block = if let Some(attestation) = self.last_sent_attestation.clone() {
            attestation.attestation_data.header_number
        } else if let Some(attestation) = last_attestation {
            attestation.attestation.header_number
        } else {
            0
        };

        // Create the fragment for the signed attestation
        // This is the continuity proof of this signed attestation
        retry::ret(
            || async {
                let mut signed_attestation = signed_attestation.clone();
                fragment::create(chain_key, start_block, &mut signed_attestation, eth_client).await
            },
            10,
            10,
            None,
        )
        .await?;

        Ok(signed_attestation)
    }
}
