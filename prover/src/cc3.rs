use anyhow::Result;
use serde::{Deserialize, Serialize};
use sp_core::H256;
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::{debug, error, info};

pub use cc_client::claim::Claim;
use cc_client::Client as CcClient;

pub type Randomness = [u8; 32];

#[derive(Debug, Clone, Serialize, Deserialize, Error)]
pub enum Error {
    #[error("Failed to submit RPC")]
    FailedToSubmit,
    #[error("Failed to sign Babe VRF output")]
    FailedToSignBabeVrf,
    #[error("Failed to check eligibility")]
    FailedToCheckEligibility,
    #[error("Failed to fetch latest digest")]
    FailedToFetchDigest,
    #[error("Invalid attestor")]
    InvalidAttestor,
    #[error("Invalid bls key")]
    InvalidBlsKey,
    #[error("Failed to get cc3 RPC client")]
    FailedToGetRPcClient,
}

#[derive(Debug, Clone)]
/// Cc3 client that is configured with an url and keypair
/// Must connect to a node that has rpc and websocket enabled
/// - `cc_client`: Creditcoin3 client
pub struct Client {
    pub cc_client: CcClient,
    pub nickname: String,
}

impl<'a> Client {
    /// Create a new instance of cc3 client
    /// - `url`: rpc url of a creditcoin node
    /// - `key`: secret phrase for a creditcoin key
    /// - `nickname`: nickname for this prover
    pub fn new(
        url: impl Into<String> + Clone,
        key: &'a str,
        nickname: impl Into<String> + Clone,
    ) -> Result<Self> {
        let cc_client = CcClient::new(url, key)?;

        Ok(Self {
            cc_client,
            nickname: nickname.into(),
        })
    }

    /// Init the client, this bootstraps prover registration if not registered already
    pub async fn init(&self) -> Result<()> {
        let is_member = self.cc_client.check_provers_membership().await?;

        if !is_member {
            debug!("prover registration in progress... Please wait...");
            self.register().await?;
        }

        info!("Prover ready to start!");

        Ok(())
    }

    /// Register to the prover pallet
    pub async fn register(&self) -> Result<()> {
        self.cc_client.register_prover(self.nickname.clone()).await
    }

    pub async fn submit_proof(&self, claim_hash: H256, proof: Vec<u8>) -> Result<()> {
        info!("Submitting proof len: {}", proof.len());

        self.cc_client.submit_proof(claim_hash, proof).await
    }
}

impl Client {
    pub async fn start_claim_sub(
        &self,
        mut cancel: tokio::sync::oneshot::Receiver<()>,
        claim_chan: mpsc::Sender<Claim>,
    ) -> Result<()> {
        let mut subscription = self.cc_client.subscribe_claim_submission_events().await?;

        // Process claims in a loop
        loop {
            tokio::select! {
                claim = subscription.next() => {
                    match claim {
                        Some(claim) => {
                            // Process the claim
                            info!("Received a new claim: {:?}", claim);
                            // Handle the claim processing logic here
                            claim_chan.send(claim).await?;
                        }
                        None => break, // Exit loop if the subscription stream ends
                    }
                }
                rec = &mut cancel => {
                    if let Ok(()) = rec { panic!("This doesn't happen") } else {
                        info!("Cancellation received, stopping claim processing");
                        break;
                    }
                }
            }
        }

        Ok(())
    }
}
