use alloy::primitives::Address;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sp_core::{H256, U256};
use std::str::FromStr;
use subxt::backend::rpc::{RpcClient, RpcParams};
pub use subxt::utils::AccountId32;
use subxt::{config::DefaultExtrinsicParamsBuilder, OnlineClient, SubstrateConfig};
use subxt_signer::{
    sr25519::{Keypair, Signature},
    SecretUri,
};
use thiserror::Error;
use tracing::{debug, error, info};

use cc3::runtime_types::attestor_primitives::{
    Attestation as CcAttestation, SignedAttestation as CcSignedAttestation,
};
use cc3::runtime_types::prover_primitives::ChainPriceConfiguration;

use attestor_primitives::{
    Attestation, BlsPublicKey, BlsSignature, ChainId, Digest, SignedAttestation,
};
use creditcoin3_attestor_gossip::{Attestation as RpcAttestation, AttestorId, VrfOutput};

#[subxt::subxt(
    runtime_metadata_path = "artifacts/metadata.scale",
    substitute_type(
        path = "primitive_types::U256",
        with = "::subxt::utils::Static<crate::U256>"
    )
)]

pub mod cc3 {}

pub mod attestation;
pub mod claim;
pub mod proof;

use cc3::runtime_types::pallet_prover::types::Prover;

pub type Randomness = [u8; 32];

#[derive(Debug, Clone)]
/// Cc3 client that is configured with an url and keypair
/// Must connect to a node that has rpc and websocket enabled
/// - `url`: Creditcoin3 url (rpc + websocket enabled)
/// - `keypair`: Creditcoin3 keypair
pub struct Client {
    url: String,
    pub keypair: Keypair,
    pub evm_address: Address,
    api: OnlineClient<SubstrateConfig>,
}

impl<'a> Client {
    /// Create a new instance of cc3 client
    /// - `url`: rpc url of a creditcoin node
    /// - `key`: secret phrase for a creditcoin key
    pub async fn new(
        url: impl Into<String> + Clone,
        key: &'a str,
        // private_key: &[u8; 32],
    ) -> Result<Self> {
        let secret_uri = SecretUri::from_str(key)?;
        let keypair = Keypair::from_uri(&secret_uri)?;

        let evm_address_slice = &keypair.public_key().0[0..20];
        // let a = hex::encode(evm_address_slice);
        let evm_address = Address::from_slice(evm_address_slice);
        info!("Substrate evm address: {:?}", evm_address);

        let url = url.into();
        let api = OnlineClient::<SubstrateConfig>::from_url(&url).await?;

        Ok(Self {
            url,
            keypair,
            evm_address,
            api,
        })
    }

    #[must_use]
    pub fn sign(&self, message: &[u8]) -> Signature {
        self.keypair.sign(message)
    }

    #[must_use]
    pub fn get_evm_address(&self) -> Address {
        self.evm_address
    }

    pub async fn get_chain_key(
        url: impl Into<String> + Clone,
        chain_id: u64,
        name: Vec<u8>,
    ) -> Result<Option<ChainId>> {
        let url = url.into();
        let api = OnlineClient::<SubstrateConfig>::from_url(&url).await?;
        let chain_key = api
            .storage()
            .at_latest()
            .await?
            .fetch(
                &cc3::storage()
                    .supported_chains()
                    .chain_id_and_name_to_uniq_key(chain_id, name),
            )
            .await?;

        Ok(chain_key)
    }

    /// Fetches the babe randomness from 2 epochs ago
    /// Returns the random a time + the current block number (where it was calculated from)
    pub(crate) async fn fetch_babe_randomness(&self) -> Result<(Randomness, u64)> {
        let epoch_index = self
            .api
            .storage()
            .at_latest()
            .await?
            .fetch(&cc3::storage().babe().epoch_index())
            .await?;

        // Calculate the epoch index we are interested in
        // This is the current epoch index - 2
        let intrested_epoch_index = epoch_index.unwrap_or(0).saturating_sub(2);

        // Short circuit if epoch index is too low
        // Randomness is not available for the first 2 epochs
        if intrested_epoch_index < 2 {
            tracing::info!("Epoch index is too low to fetch randomness");
            return Ok((Randomness::default(), intrested_epoch_index));
        }

        let randomness = self
            .api
            .storage()
            .at_latest()
            .await?
            .fetch(
                &cc3::storage()
                    .randomness()
                    .randomness_by_epoch_index(intrested_epoch_index),
            )
            .await?
            .ok_or(Error::FailedToGetBabeVrf)?;

        Ok((randomness, intrested_epoch_index))
    }

    pub async fn _fetch_comittee_size(&self) -> Result<u32> {
        let storage_query = cc3::storage().attestation().comittee_set_size();

        let result = self
            .api
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?
            .ok_or(Error::FailedToGetComitteSetSize)?;

        Ok(result)
    }

    pub async fn fetch_last_digest(&self, chain_id: ChainId) -> Result<Option<Digest>> {
        let storage_query = cc3::storage().attestation().last_digest(chain_id);

        let result = self
            .api
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        Ok(result)
    }

    /// Check the clients membership in the attestor pallet
    pub async fn check_attestors_membership(&self) -> Result<bool> {
        let storage_query = cc3::storage()
            .attestation()
            .attestors(subxt::utils::AccountId32::from(self.keypair.public_key()));

        let result = self
            .api
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?
            .is_some();

        Ok(result)
    }

    /// Register to the attestation pallet
    pub async fn register_attestor(
        &self,
        bls_public_key: BlsPublicKey,
        proof_of_possession: BlsSignature,
    ) -> Result<()> {
        let tx = cc3::tx()
            .attestation()
            .register_attestor(bls_public_key, proof_of_possession);

        let ext = self
            .api
            .tx()
            .sign_and_submit_then_watch_default(&tx, &self.keypair)
            .await?
            .wait_for_finalized_success()
            .await?;

        let hash = ext.extrinsic_hash();
        debug!("Registration extrinsic submitted with hash: {:?}", hash);

        Ok(())
    }

    /// Check the clients membership in the prover pallet
    pub async fn check_provers_membership(&self, account_id: Option<AccountId32>) -> Result<bool> {
        let account_id = if let Some(account_id) = account_id {
            account_id
        } else {
            AccountId32(self.keypair.public_key().0)
        };

        let storage_query = cc3::storage().prover().provers(account_id);

        let result = self
            .api
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?
            .is_some();

        Ok(result)
    }

    /// Register to the prover pallet
    pub async fn register_prover(&self, nickname: String) -> Result<()> {
        let tx = cc3::tx().prover().register_prover(Prover {
            nickname: nickname.into(),
        });

        let ext = self
            .api
            .tx()
            .sign_and_submit_then_watch_default(&tx, &self.keypair)
            .await?
            .wait_for_finalized_success()
            .await?;

        let hash = ext.extrinsic_hash();
        debug!("Registration extrinsic submitted with hash: {:?}", hash);

        Ok(())
    }

    /// `sign_babe_vrf` signs babe's author vrf randomness with the configured key and returns the output as integer
    /// the method extracts the S component bytes from the signature. The bytes of the S component are converted into a u64 integer using little-endian byte order.
    pub async fn sign_babe_vrf(&self) -> Result<VrfOutput, Error> {
        let (randomness, epoch_index) = self.fetch_babe_randomness().await.map_err(|e| {
            error!("Error getting babe vrf output: {:?}", e);
            Error::FailedToGetBabeVrf
        })?;

        info!("Babe VRF Randomness: {}", hex::encode(randomness));

        let randomness_as_u256 = U256::from_little_endian(&randomness);

        // Sign the randomness
        let signature = self.keypair.sign(&randomness);

        // Convert `S` component bytes to a [u8; 32] array
        let mut s_component_array = [0; 32];
        s_component_array.copy_from_slice(&signature.0[32..64]);

        // Convert `S` component bytes to an integer
        let signature_output_as_u256 = U256::from_little_endian(&s_component_array);

        info!(
            "Signature output is above or below threshold: {}",
            signature_output_as_u256 > randomness_as_u256
        );

        Ok(VrfOutput {
            signature: sp_core::sr25519::Signature::from_raw(signature.0),
            vrf_number: sp_core::U256::from_little_endian(&s_component_array),
            epoch: epoch_index,
        })
    }

    #[must_use]
    pub fn get_attestor_id(&self) -> AttestorId {
        AttestorId::from_public(self.keypair.public_key().0)
    }

    pub async fn get_chain_price_configurations(
        &self,
        account_id: Option<AccountId32>,
    ) -> Result<Vec<ChainPriceConfiguration>> {
        let account_id = if let Some(account_id) = account_id {
            account_id
        } else {
            AccountId32(self.keypair.public_key().0)
        };

        let storage_query = cc3::storage()
            .prover()
            .provers_chain_price_configurations(account_id);

        let result = self
            .api
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?
            .ok_or(Error::FailedToGetChainPriceConfigurations)?;

        Ok(result)
    }

    pub async fn update_chain_price_configurations(
        &self,
        chain_price_configurations: Vec<ChainPriceConfiguration>,
    ) -> Result<()> {
        let tx = cc3::tx()
            .prover()
            .set_chain_price_config(chain_price_configurations);

        let ext = self
            .api
            .tx()
            .sign_and_submit_then_watch_default(&tx, &self.keypair)
            .await?
            .wait_for_finalized_success()
            .await?;

        let hash = ext.extrinsic_hash();
        debug!(
            "Chain price configurations extrinsic submitted with hash: {:?}",
            hash
        );

        Ok(())
    }

    pub async fn chain_attestation_interval(&self, chain_id: ChainId) -> Result<Option<u64>> {
        let storage_query = cc3::storage()
            .attestation()
            .chain_attestation_interval(chain_id);

        let result = self
            .api
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        Ok(result)
    }

    pub async fn chain_attestation_exists(
        &self,
        chain_id: ChainId,
        digest: Digest,
    ) -> Result<bool> {
        let storage_query = cc3::storage().attestation().attestations(chain_id, digest);

        let result = self
            .api
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        Ok(result.is_some())
    }

    pub async fn get_attestation_by_digest(
        &self,
        chain_id: ChainId,
        digest: Digest,
    ) -> Result<Option<SignedAttestation<H256, AccountId32>>> {
        let storage_query = cc3::storage().attestation().attestations(chain_id, digest);

        let result = self
            .api
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        Ok(result.map(Into::into))
    }

    pub async fn submit_attestation<H, A>(&self, attestation: RpcAttestation<H, A>) -> Result<()>
    where
        H: Serialize,
        A: Serialize,
    {
        let rpc_client = RpcClient::from_url(self.url.clone()).await?;

        let mut params = RpcParams::new();
        params.push(attestation)?;

        rpc_client
            .request::<()>("attestor_submitAttestation", params)
            .await?;

        info!("Attestation submitted");
        Ok(())
    }

    pub async fn transfer(
        &self,
        target: AccountId32,
        amount: u64,
        account_nonce: Option<u64>,
    ) -> Result<()> {
        let tx = cc3::tx()
            .balances()
            .transfer(subxt::utils::MultiAddress::Id(target), amount.into());

        let params = if let Some(account_nonce) = account_nonce {
            DefaultExtrinsicParamsBuilder::new()
                .nonce(account_nonce)
                .build()
        } else {
            DefaultExtrinsicParamsBuilder::new().build()
        };

        let ext = self
            .api
            .tx()
            .create_signed_offline(&tx, &self.keypair, params)?
            .submit_and_watch()
            .await?;

        // let hash = ext.extrinsic_hash();
        debug!("Transfer extrinsic submitted with hash: {:?}", ext);

        Ok(())
    }

    pub async fn get_account_nonce(&self) -> Result<u64> {
        let nonce = self
            .api
            .tx()
            .account_nonce(&AccountId32(self.keypair.public_key().0))
            .await?;

        Ok(nonce)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct ChainPriceConfig {
    pub chain_id: ChainId,
    pub price: u64,
}

impl From<ChainPriceConfiguration> for ChainPriceConfig {
    fn from(config: ChainPriceConfiguration) -> Self {
        ChainPriceConfig {
            chain_id: config.chain_id,
            price: config.price,
        }
    }
}

impl From<ChainPriceConfig> for ChainPriceConfiguration {
    fn from(val: ChainPriceConfig) -> Self {
        ChainPriceConfiguration {
            chain_id: val.chain_id,
            price: val.price,
        }
    }
}

impl<H, A> From<CcSignedAttestation<H, A>> for SignedAttestation<H, A>
where
    H: Into<H256>,
{
    fn from(attestation: CcSignedAttestation<H, A>) -> Self {
        SignedAttestation {
            attestation: attestation.attestation.into(),
            signature: attestation.signature,
            attestors: attestation.attestors,
        }
    }
}

impl<H> From<CcAttestation<H>> for Attestation<H>
where
    H: Into<H256>,
{
    fn from(attestation: CcAttestation<H>) -> Self {
        Attestation {
            chain_id: attestation.chain_id,
            header_number: attestation.header_number,
            header_hash: attestation.header_hash,
            tx_root: attestation.tx_root,
            rx_root: attestation.rx_root,
            prev_digest: attestation.prev_digest.map(Into::into),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Error)]
pub enum Error {
    #[error("Cannot attest")]
    CannotAttest,
    #[error("Failed to submit RPC")]
    FailedToSubmit,
    #[error("Failed to get Babe VRF")]
    FailedToGetBabeVrf,
    #[error("Failed to get block number")]
    FailedToGetBlockNumber,
    #[error("Babe VRF output is invalid")]
    BabeVrfOuputInvalid,
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
    #[error("Invalid proof of possession")]
    InvalidProofOfPossession,
    #[error("Failed to get cc3 RPC client")]
    FailedToGetRPcClient,
    #[error("Failed to get comittee set size")]
    FailedToGetComitteSetSize,
    #[error("Failed to get chain price configurations")]
    FailedToGetChainPriceConfigurations,
    #[error("Failed to get attestation interval")]
    FailedToGetAttestationInterval,
}
