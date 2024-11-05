use anyhow::Result;
use serde::Serialize;
use sp_core::{
    sr25519::{self},
    Pair, H256, U256,
};
use std::str::FromStr;
pub use subxt::utils::AccountId32;
pub use subxt::Error::Rpc as SubxtRpcError;
use subxt::{
    backend::rpc::{RpcClient, RpcParams},
    error::RpcError,
    ext::futures::StreamExt,
};
use subxt::{config::DefaultExtrinsicParamsBuilder, OnlineClient, SubstrateConfig};
use subxt_signer::{
    sr25519::{Keypair, Signature},
    SecretUri,
};
use thiserror::Error;
use tracing::{debug, error, info, warn};

use cc3::runtime_types::attestor_primitives::{
    Attestation as CcAttestation, AttestationCheckpoint as CcAttestationCheckpoint,
    SignedAttestation as CcSignedAttestation,
};

use attestor_primitives::{
    Attestation, AttestationCheckpoint, AttestorId, BlsPublicKey, BlsSignature, ChainKey, Digest,
    SignedAttestation,
};
use creditcoin3_attestor_gossip::Attestation as RpcAttestation;
use vrf::{make_proof_of_inclusion, ProofOfInclusion};

#[subxt::subxt(
    runtime_metadata_path = "artifacts/metadata.scale",
    substitute_type(
        path = "primitive_types::U256",
        with = "::subxt::utils::Static<crate::U256>"
    )
)]

pub mod cc3 {}

pub mod attestation;

pub type Randomness = [u8; 32];

#[derive(Clone)]
/// Cc3 client that is configured with an url and keypair
/// Must connect to a node that has rpc and websocket enabled
/// - `url`: Creditcoin3 url (rpc + websocket enabled)
/// - `keypair`: Creditcoin3 keypair
pub struct Client {
    url: String,
    pair: sr25519::Pair,
    signing_keypair: Keypair,
    api: OnlineClient<SubstrateConfig>,
}

impl<'a> Client {
    /// Create a new instance of cc3 client
    /// - `url`: rpc url of a creditcoin node
    /// - `key`: secret phrase for a creditcoin key
    pub async fn new(url: impl Into<String> + Clone, key: &'a str) -> Result<Self> {
        let secret_uri = SecretUri::from_str(key)?;
        let signing_keypair = Keypair::from_uri(&secret_uri)?;

        let pair = sr25519::Pair::from_string(key, None)?;

        let url = url.into();
        let api = if url.contains("ws") || url.contains("http") {
            OnlineClient::<SubstrateConfig>::from_insecure_url(&url).await?
        } else {
            OnlineClient::<SubstrateConfig>::from_url(&url).await?
        };

        Ok(Self {
            url,
            pair,
            signing_keypair,
            api,
        })
    }

    #[must_use]
    pub fn sign(&self, message: &[u8]) -> Signature {
        self.signing_keypair.sign(message)
    }

    pub async fn get_chain_key(&self, chain_id: u64, name: String) -> Result<Option<ChainKey>> {
        let chain_key = self
            .api
            .storage()
            .at_latest()
            .await?
            .fetch(
                &cc3::storage()
                    .supported_chains()
                    .chain_id_and_name_to_uniq_key(chain_id, name.as_bytes()),
            )
            .await?;

        Ok(chain_key)
    }

    /// Fetches the babe randomness from 2 epochs ago
    /// Returns the random a time + the current block number (where it was calculated from)
    pub async fn fetch_babe_randomness_two_epoch_ego(&self) -> Result<(Randomness, u64)> {
        let epoch_index = self
            .api
            .storage()
            .at_latest()
            .await?
            .fetch(&cc3::storage().babe().epoch_index())
            .await?;

        // Calculate the epoch index we are interested in
        // This is the current epoch index - 2
        let two_epoch_ago = epoch_index.unwrap_or(0).saturating_sub(2);

        // Short circuit if epoch index is too low
        // Randomness is not available for the first 2 epochs
        if two_epoch_ago == 0 {
            info!("Epoch index is too low to fetch randomness");
            return Ok((Randomness::default(), two_epoch_ago));
        }
        info!("Fetching randomness for epoch index: {}", two_epoch_ago);

        let randomness = self
            .api
            .storage()
            .at_latest()
            .await?
            .fetch(
                &cc3::storage()
                    .randomness()
                    .randomness_by_epoch_index(two_epoch_ago),
            )
            .await?
            .ok_or(Error::FailedToGetBabeVrf)?;

        Ok((randomness, two_epoch_ago))
    }

    pub async fn fetch_committee_size(&self, chain_key: u64) -> Result<u32> {
        let storage_query = cc3::storage().attestation().committee_set_size(chain_key);

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

    pub async fn fetch_last_digest(&self, chain_key: ChainKey) -> Result<Option<Digest>, Error> {
        let storage_query = cc3::storage().attestation().last_digest(chain_key);

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
    pub async fn check_attestors_membership(&self, chain_key: u64) -> Result<bool> {
        let storage_query = cc3::storage().attestation().active_attestors(chain_key);

        let result = self
            .api
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        match result {
            Some(result) => Ok(result.contains(&AccountId32(self.signing_keypair.public_key().0))),
            None => Ok(false),
        }
    }

    /// Register to the attestation pallet
    pub async fn register_attestor(
        &self,
        chain_key: u64,
        attestor_id: AccountId32,
        account_nonce: Option<u64>,
    ) -> Result<()> {
        let tx = cc3::tx()
            .attestation()
            .register_attestor(chain_key, attestor_id);

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
            .create_signed(&tx, &self.signing_keypair, params)
            .await?
            .submit_and_watch()
            .await?;

        let hash = ext.extrinsic_hash();
        debug!("Registration extrinsic submitted with hash: {:?}", hash);

        Ok(())
    }

    pub async fn start_attesting(
        &self,
        chain_key: ChainKey,
        bls_public_key: BlsPublicKey,
        proof_of_possession: BlsSignature,
    ) -> Result<()> {
        let tx = cc3::tx()
            .attestation()
            .attest(chain_key, bls_public_key, proof_of_possession);

        let ext = self
            .api
            .tx()
            .sign_and_submit_then_watch_default(&tx, &self.signing_keypair)
            .await?
            .wait_for_finalized_success()
            .await?;

        let hash = ext.extrinsic_hash();
        debug!("Start Attesting extrinsic submitted with hash: {:?}", hash);

        Ok(())
    }

    /// `sign_babe_vrf` signs babe's author vrf randomness with the configured key and returns the output as integer
    /// the method extracts the S component bytes from the signature. The bytes of the S component are converted into a u64 integer using little-endian byte order.
    pub async fn sign_babe_vrf(
        &self,
        chain_key: ChainKey,
        randomness: Randomness,
        epoch_index: u64,
    ) -> Result<ProofOfInclusion, Error> {
        // Get committee set size
        let committee_size = self.fetch_committee_size(chain_key).await.map_err(|e| {
            error!("Error getting committee size: {:?}", e);
            Error::FailedToGetComitteSetSize
        })?;

        let attestor_working_set_size = self
            .get_attestor_working_set_size(chain_key)
            .await
            .map_err(|e| {
                error!("Error getting attestor working set size: {:?}", e);
                Error::FailedToGetAttestorWorkingSetSize
            })?;

        info!("Committee set size: {}", committee_size);
        info!("Attestor working set size: {}", attestor_working_set_size);

        let proof_of_inclusion = make_proof_of_inclusion(
            attestor_working_set_size as u64,
            u64::from(committee_size),
            &randomness,
            &self.pair,
            &self.get_attestor_id(),
            epoch_index,
        )
        .map_err(|e| {
            error!("Error creating proof of inclusion: {:?}", e);
            Error::FailedToGetBabeVrf
        })?;

        Ok(proof_of_inclusion)
    }

    #[must_use]
    pub fn get_attestor_id(&self) -> AttestorId {
        AttestorId::from_public(self.signing_keypair.public_key().0)
    }

    pub async fn chain_attestation_interval(&self, chain_key: ChainKey) -> Result<Option<u64>> {
        let storage_query = cc3::storage()
            .attestation()
            .chain_attestation_interval(chain_key);

        let result = self
            .api
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        Ok(result)
    }

    pub async fn chain_checkpoint_interval(&self, chain_key: ChainKey) -> Result<Option<u32>> {
        let storage_query = cc3::storage()
            .attestation()
            .attestation_checkpoint_interval(chain_key);

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
        chain_key: ChainKey,
        digest: Digest,
    ) -> Result<bool, Error> {
        let storage_query = cc3::storage().attestation().attestations(chain_key, digest);

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
        chain_key: ChainKey,
        digest: Digest,
    ) -> Result<Option<SignedAttestation<H256, AccountId32>>, Error> {
        let storage_query = cc3::storage().attestation().attestations(chain_key, digest);

        let result = self
            .api
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        Ok(result.map(Into::into))
    }

    pub async fn get_checkpoint_by_digest(
        &self,
        chain_key: ChainKey,
        digest: Digest,
    ) -> Result<Option<AttestationCheckpoint>> {
        let storage_query = cc3::storage().attestation().checkpoints(chain_key, digest);

        let result = self
            .api
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        Ok(result.map(Into::into))
    }

    pub async fn get_attestations_for_chain(
        &self,
        chain_key: ChainKey,
    ) -> Result<Vec<SignedAttestation<H256, AccountId32>>> {
        let mut attestations = Vec::new();

        // Address to the root of a storage entry that we'd like to iterate over
        // concatenated with the encoded first key to the Attestations double map,
        // a ChainKey
        let address = cc3::storage().attestation().attestations_iter1(chain_key);

        let mut iter = self.api.storage().at_latest().await?.iter(address).await?;

        while let Some(Ok(kv)) = iter.next().await {
            attestations.push(kv.value.into());
        }

        attestations.sort_by(
            |a: &SignedAttestation<H256, AccountId32>, b: &SignedAttestation<H256, AccountId32>| {
                // Highest to lowest by comparing b to a
                b.attestation
                    .header_number
                    .cmp(&a.attestation.header_number)
            },
        );

        Ok(attestations)
    }

    pub async fn get_checkpoints_for_chain(
        &self,
        chain_key: ChainKey,
    ) -> Result<Vec<AttestationCheckpoint>> {
        let mut checkpoints = Vec::new();

        // Address to the root of a storage entry that we'd like to iterate over
        // concatenated with the encoded first key to the Checkpoints double map,
        // a ChainKey.
        let address = cc3::storage().attestation().checkpoints_iter1(chain_key);

        let mut iter = self.api.storage().at_latest().await?.iter(address).await?;

        while let Some(Ok(kv)) = iter.next().await {
            checkpoints.push(kv.value.into());
        }

        checkpoints.sort_by(|a: &AttestationCheckpoint, b: &AttestationCheckpoint| {
            // Highest to lowest by comparing b to a
            b.block_number.cmp(&a.block_number)
        });

        Ok(checkpoints)
    }

    pub async fn submit_attestation<H, A>(
        &self,
        attestation: RpcAttestation<H, A>,
    ) -> Result<(), Error>
    where
        H: Serialize,
        A: Serialize,
    {
        let rpc_client = if self.url.contains("http") || self.url.contains("ws") {
            warn!("Creating insucure rpc client to submit attestation");
            RpcClient::from_insecure_url(self.url.clone())
                .await
                .map_err(|e| {
                    error!("Error creating insecure rpc client: {:?}", e);
                    Error::InvalidUrl
                })?
        } else {
            RpcClient::from_url(self.url.clone()).await.map_err(|e| {
                error!("Error creating rpc client: {:?}", e);
                Error::InvalidUrl
            })?
        };

        let mut params = RpcParams::new();
        params
            .push(attestation)
            .map_err(|_| Error::FailedToSubmit)?;

        match rpc_client
            .request::<()>("attestor_submitAttestation", params)
            .await
        {
            Ok(()) => {
                info!("Attestation submitted");
                Ok(())
            }
            Err(e) => {
                if let subxt::Error::Rpc(e) = e {
                    error!("Error submitting attestation: {:?}", e);
                    Err(Error::RpcError(e))
                } else {
                    error!("Error submitting attestation: {:?}", e);
                    Err(Error::FailedToSubmit)
                }
            }
        }
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
            .create_signed(&tx, &self.signing_keypair, params)
            .await?
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
            .account_nonce(&AccountId32(self.signing_keypair.public_key().0))
            .await?;

        Ok(nonce)
    }

    pub async fn get_attestor_working_set_size(&self, chain_key: u64) -> Result<usize> {
        let address = cc3::storage().attestation().attestors_iter1(chain_key);

        let count = self
            .api
            .storage()
            .at_latest()
            .await?
            .iter(address)
            .await?
            .count()
            .await;

        Ok(count)
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
            chain_key: attestation.chain_key,
            header_number: attestation.header_number,
            header_hash: attestation.header_hash,
            root: attestation.root,
            prev_digest: attestation.prev_digest.map(Into::into),
        }
    }
}

impl From<CcAttestationCheckpoint> for AttestationCheckpoint {
    fn from(checkpoint: CcAttestationCheckpoint) -> Self {
        AttestationCheckpoint {
            block_number: checkpoint.block_number,
            digest: checkpoint.digest,
        }
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Cannot attest")]
    CannotAttest,
    #[error("Failed to submit RPC")]
    FailedToSubmit,
    #[error("Attetation not included in block")]
    AttestationNotIncluded,
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
    #[error("Failed to get committee set size")]
    FailedToGetComitteSetSize,
    #[error("Failed to get chain price configurations")]
    FailedToGetChainPriceConfigurations,
    #[error("Failed to get attestation interval")]
    FailedToGetAttestationInterval,
    #[error("Failed to get chain key")]
    FailedToGetChainKey,
    #[error("Failed to attestor working set size")]
    FailedToGetAttestorWorkingSetSize,
    #[error("Failed to get attestation for last digest")]
    LastAttestationNotFound,
    #[error("Subxt error: {0:?}")]
    SubxtError(#[from] subxt::Error),
    #[error("Rpc error: {0:?}")]
    RpcError(#[from] RpcError),
    #[error("Invalid rpc url")]
    InvalidUrl,
}
