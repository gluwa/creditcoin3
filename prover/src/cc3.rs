use anyhow::Result;
use attestor_primitives::{ChainId, Digest, SignedAttestation};
use pallet_evm::{AddressMapping as EvmAddressMapping, HashedAddressMapping};
use serde::{Deserialize, Serialize};
use sp_core::{Blake2Hasher, H160, H256};
use std::str::FromStr;
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::{debug, error, info};

use crate::config::Chain;

pub use cc_client::{
    claim::{AccountId32, Claim},
    ChainPriceConfig, Client as CcClient,
};

type AddressMapping = HashedAddressMapping<Blake2Hasher>;

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
    #[error("Failed parse key")]
    KeyError,
}

#[derive(Debug, Clone)]
/// Cc3 client that is configured with an url and keypair
/// Must connect to a node that has rpc and websocket enabled
/// - `cc_client`: Creditcoin3 client
/// - `nickname`: nickname for this prover
pub struct Client {
    cc_client: CcClient,
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
        let account_id: AccountId32 = eth_acct(self.cc_client.get_evm_address().into_array())?;

        let is_member = self
            .cc_client
            .check_provers_membership(Some(account_id))
            .await?;

        if !is_member {
            debug!("prover registration in progress... Please wait...");
            self.register().await?;
        }

        info!("Prover ready to start!");

        Ok(())
    }

    /// Register to the prover pallet
    pub async fn register(&self) -> Result<()> {
        self.cc_client.register(self.nickname.clone()).await
    }

    /// Submit a proof to the prover pallet for a given hash
    /// - `claim_hash`: hash of the claim
    /// - `proof`: proof bytes
    pub async fn submit_proof(&self, claim_hash: H256, proof: Vec<u8>) -> Result<()> {
        info!("Submitting proof len: {}", proof.len());

        self.cc_client.submit_proof(claim_hash, proof).await
    }

    /// Sync chain prices configuration
    /// - `client`: cc3 client
    /// - `config_chain_prices`: chain price configurations
    pub async fn sync_chain_prices_configuration(
        &self,
        config_chain_prices: Vec<Chain>,
    ) -> Result<()> {
        let account_id: AccountId32 = eth_acct(self.cc_client.get_evm_address().into_array())?;

        let chain_price_configurations: Vec<ChainPriceConfig> = self
            .cc_client
            .get_chain_price_configurations(Some(account_id))
            .await?
            .into_iter()
            .map(std::convert::Into::into)
            .collect();

        info!(
            "Syncing chain price configurations: {:?}",
            chain_price_configurations
        );

        // TODO: compare with current configuration and update if needed
        let config_chain_prices: Vec<ChainPriceConfig> = config_chain_prices
            .into_iter()
            .map(std::convert::Into::into)
            .collect();

        // compare with current configuration and update if needed
        if chain_price_configurations == config_chain_prices {
            info!("Chain price configurations are up to date");
        } else {
            info!("Updating chain price configurations");
            self.cc_client
                .set_chain_price_config(
                    config_chain_prices
                        .into_iter()
                        .map(std::convert::Into::into)
                        .collect(),
                )
                .await?;
        };

        Ok(())
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
}

impl Client {
    pub async fn start_claim_sub(
        &self,
        mut cancel: tokio::sync::oneshot::Receiver<()>,
        claim_chan: mpsc::Sender<Claim>,
    ) -> Result<()> {
        let account_id: AccountId32 = eth_acct(self.cc_client.get_evm_address().into_array())?;

        let mut subscription = self
            .cc_client
            .subscribe_claim_submission_events(Some(account_id))
            .await?;

        // Process claims in a loop
        loop {
            tokio::select! {
                claim = subscription.next() => {
                    match claim {
                        Some(claim) => {
                            // Process the claim
                            info!("Received a new claim: hash({:?})", claim.hash);
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

    pub async fn start_attestation_sub(
        &self,
        mut cancel: tokio::sync::oneshot::Receiver<()>,
        attestation_chan: mpsc::Sender<SignedAttestation<H256, AccountId32>>,
        filter: Vec<ChainId>,
    ) -> Result<()> {
        let mut subscription = self
            .cc_client
            .subscribe_attestations_submissions(filter)
            .await?;

        // Process claims in a loop
        loop {
            tokio::select! {
                attestation = subscription.next() => {
                    match attestation {
                        Some(attestation) => {
                            // Process the claim
                            info!("Received a new attestation: digest({:?})", attestation.digest());
                            // Handle the claim processing logic here
                            attestation_chan.send(attestation).await?;
                        }
                        None => break, // Exit loop if the subscription stream ends
                    }
                }
                rec = &mut cancel => {
                    if let Ok(()) = rec { panic!("This doesn't happen") } else {
                        info!("Cancellation received, stopping attestation processing");
                        break;
                    }
                }
            }
        }

        Ok(())
    }
}

/// Convert an Ethereum address to an `AccountId32`
/// - `b`: Ethereum address
fn eth_acct(b: [u8; 20]) -> Result<AccountId32> {
    let a: sp_core::crypto::AccountId32 = AddressMapping::into_account_id(H160::from(b));

    Ok(AccountId32::from_str(&a.to_string()).map_err(|_| Error::KeyError)?)
}
