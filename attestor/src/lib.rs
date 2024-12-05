use anyhow::Result;
use eth::Client;
use sp_core::H256;
use std::{thread, time::Duration};
use tracing::{debug, error, info, warn};

pub mod attestation;
pub mod cc3;
pub mod engine;
pub mod eth_sub;
pub mod fragment;
pub mod merkle;

use attestor_primitives::{Attestation as AttestationPrimitive, AttestorId};
use cc3::Error;
use cc_client::attestation::CcEvent;
use creditcoin3_attestor_gossip::Attestation;

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
        let eth_client = Client::new(&self.config.eth_rpc_url, &String::new()).await?;
        let chain_id = eth_client.get_chain_id().await?;
        debug!("Opened connection to ethereum chain with id {}", chain_id);

        let cc3_client =
            cc3::Client::new(&self.config.cc3_rpc_url, &self.config.cc3_key, chain_id).await?;
        cc3_client.init().await?;

        // Construct the attestation engine
        let mut engine = engine::Engine::new(eth_client, cc3_client);

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
        engine.start().await?;

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
                            info!("Randomness changed. Epoch: {:?}, Randomness: {:?}", epoch, randomness);
                            engine.evaluate().await?;
                        }
                        CcEvent::AttestationIntervalChanged(_, new_interval) => {
                            info!(
                                "Attestation interval updated. New interval: {:?}", new_interval
                            );
                            engine.change_interval(new_interval);
                        },
                        CcEvent::CheckpointReached(checkpoint, ck) => {
                            info!("Checkpoint reached: {:?}", checkpoint);
                            if chain_key != ck {
                                return Ok(());
                            }
                            engine.evaluate().await?;
                        }
                        CcEvent::BlockAttested(_) => ()
                    }
                },
                // Poll the attestation engine for the next attestation
                Some(attestation_to_submit) = engine.next() => {
                    match self.handle_attestation(chain_key, engine.eth_client(), engine.cc_client(), attestation_to_submit).await {
                        Ok(attestation) => {
                            let digest = attestation.digest();
                            info!("Submitting attestation with digest: {:?}",digest);
                            // Submit the attestation to the chain
                            engine.cc_client().submit_attestation::<H256>(attestation.clone()).await?;
                            self.last_sent_attestation = Some(attestation);
                            info!("Attestation with digest: {:?} submitted", digest);
                        },
                        Err(e) => {
                            if e.is_not_selected_error() {
                                warn!("Failed to create proof of inclusion, attestor not selected. Nothing to do here, waiting for next attestation");
                            } else if e.is_duplicate_submission() {
                                warn!("Attestation already submitted, skipping");
                            } else {
                                error!("Non-retryable error submitting attestation: {:?}", e);
                                return Err(e.into());
                            }

                        }
                    };

                    // Sleep for a while to avoid spamming the chain
                    thread::sleep(Duration::from_secs(6));
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
        let mut signed_attestation = cc3_client.sign_attestation(attestation).await?;

        // Exit early if the attestation has already been submitted
        if let Some(last_sent_attestation) = self.last_sent_attestation.clone() {
            if last_sent_attestation.attestation_data.header_number
                >= signed_attestation.attestation_data.header_number
            {
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
        info!("fragment start block: {start_block}");

        // Create the fragment for the signed attestation
        // This is the continuity proof of this signed attestation
        fragment::create(chain_key, start_block, &mut signed_attestation, eth_client).await?;

        Ok(signed_attestation)
    }
}
