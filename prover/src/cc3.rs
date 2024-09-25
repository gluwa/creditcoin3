use anyhow::Result;
use attestor_primitives::{AttestationCheckpoint, ChainId, Digest, SignedAttestation};
use serde::{Deserialize, Serialize};
use sp_core::H256;
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::{error, info};

pub use cc_client::{attestation::CcEvent, AccountId32, Client as CcClient};

// pub type Randomness = [u8; 32];

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
    #[error("Failed parse key")]
    Key,
    #[error("Unsupported chain")]
    UnsupportedChain,
}

#[derive(Clone)]
/// Cc3 client that is configured with an url and keypair
/// Must connect to a node that has rpc and websocket enabled
/// - `cc_client`: Creditcoin3 client
pub struct Client {
    cc_client: CcClient,
}

impl<'a> Client {
    /// Create a new instance of cc3 client
    /// - `url`: rpc url of a creditcoin node
    /// - `key`: secret phrase for a creditcoin key
    /// - `nickname`: nickname for this prover
    pub async fn new(url: impl Into<String> + Clone, key: &'a str) -> Result<Self> {
        let cc_client = CcClient::new(url, key).await?;

        Ok(Self { cc_client })
    }

    pub async fn fetch_last_digest(&self, chain_id: ChainId) -> Result<Option<Digest>> {
        self.cc_client.fetch_last_digest(chain_id).await
    }

    pub async fn get_attestation_by_digest(
        &self,
        chain_id: ChainId,
        digest: Digest,
    ) -> Result<Option<SignedAttestation<H256, AccountId32>>> {
        self.cc_client
            .get_attestation_by_digest(chain_id, digest)
            .await
    }

    pub async fn get_checkpoint_by_digest(
        &self,
        chain_id: ChainId,
        digest: Digest,
    ) -> Result<Option<AttestationCheckpoint>> {
        self.cc_client
            .get_checkpoint_by_digest(chain_id, digest)
            .await
    }

    pub async fn get_attestation_chain_interval(&self, chain_id: ChainId) -> Result<Option<u64>> {
        self.cc_client.chain_attestation_interval(chain_id).await
    }

    pub async fn get_chain_key(&self, chain_id: ChainId) -> Result<Option<u64>> {
        match chain_id {
            1 => {
                self.cc_client
                    .clone()
                    .get_chain_key(chain_id, "Ethereum".to_string())
                    .await
            }
            31337 => {
                self.cc_client
                    .clone()
                    .get_chain_key(chain_id, "Local anvil".to_string())
                    .await
            }
            11_155_111 => {
                self.cc_client
                    .clone()
                    .get_chain_key(chain_id, "Sepolia".to_string())
                    .await
            }
            _ => Err(Error::UnsupportedChain.into()),
        }
    }
}

impl Client {
    pub async fn start_attestation_sub(
        &self,
        attestation_chan: mpsc::UnboundedSender<SignedAttestation<H256, AccountId32>>,
        checkpoint_chan: mpsc::UnboundedSender<(AttestationCheckpoint, ChainId)>,
        filter: ChainId,
    ) -> Result<()> {
        let mut subscription = self.cc_client.subscribe_events(filter)?;

        // Process attestations in a loop
        loop {
            let event = subscription.next().await;
            match event {
                Some(CcEvent::BlockAttestedEvent(attestation)) => {
                    // Process the attestation
                    info!(
                        "Received a new attestation: chain: {}, blocknumber: {}, digest({:?})",
                        attestation.chain_id(),
                        attestation.header_number(),
                        attestation.digest()
                    );
                    // Handle the claim processing logic here
                    attestation_chan.send(attestation)?;
                }
                Some(CcEvent::CheckpointReachedEvent(checkpoint, chain_id)) => {
                    info!(
                        "Received a new attestation checkpoint: chain: {}, blocknumber: {}, digest({:?})",
                        chain_id,
                        checkpoint.block_number,
                        checkpoint.digest,
                    );
                    // Handle processing checkpoint here
                    checkpoint_chan.send((checkpoint, chain_id))?;
                }
                _ => (),
            }
        }
    }
}
