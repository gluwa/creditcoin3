use std::{str::FromStr, time::Duration};

use anyhow::Result;
use sc_service::GenericChainSpec;
use serde::Serialize;
use sp_core::{
    sr25519::{self},
    Pair, U256,
};
pub use subxt::utils::{AccountId32, H256};
use subxt::{
    backend::rpc::{
        reconnecting_rpc_client::{ExponentialBackoff, RpcClient as ReconnectionRpcClient},
        RpcParams,
    },
    config::DefaultExtrinsicParamsBuilder,
    error::RpcError,
    utils::fetch_chainspec_from_rpc_node,
    OnlineClient, SubstrateConfig,
};
use subxt_signer::{
    sr25519::{Keypair, Signature},
    SecretUri,
};
use thiserror::Error;
use tracing::{debug, error, info};

use cc3::runtime_types::{
    attestor_primitives::{
        attestation_fragment::AttestationFragmentSerializable as CcAttestationFragment,
        block::BlockSerializable as CcBlockSerializable, Attestation as CcAttestation,
        AttestationCheckpoint as CcAttestationCheckpoint,
        ChainEncodingVersion as CcChainEncodingVersion, SignedAttestation as CcSignedAttestation,
    },
    supported_chains_primitives::SupportedChain as CcSupportedChain,
};

use attestor_primitives::{
    attestation_fragment::{AttestationFragment, AttestationFragmentSerializable},
    block::Block,
    Attestation, AttestationCheckpoint, AttestorId, AttestorStatus, BlsPublicKey, BlsSignature,
    ChainEncodingVersion, ChainKey, Digest, SignedAttestation,
};
use creditcoin3_attestor_gossip::communication::Attestation as RpcAttestation;
use supported_chains_primitives::SupportedChain;
use vrf::{make_proof_of_inclusion, Error as VrfError, ProofOfInclusion};

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

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to submit RPC")]
    FailedToSubmit,
    #[error("Failed to get Babe VRF at epoch {0}")]
    FailedToGetBabeVrf(u64),
    #[error("Failed to get committee set size")]
    FailedToGetComitteSetSize,
    #[error("No Checkpoint interval set for chain with key {0}")]
    NoCheckpointIntervalSet(ChainKey),
    #[error("No Vote acceptance window set for chain with key {0}")]
    NoVoteAcceptanceWindowSet(ChainKey),
    #[error("Subxt error: {0:?}")]
    SubxtError(#[from] subxt::Error),
    #[error("Rpc error: {0:?}")]
    RpcError(#[from] RpcError),
    #[error("Invalid rpc url")]
    InvalidUrl,
    #[error("Failed to create proof of inclusion")]
    FailedToCreateProofOfInclusion(#[from] VrfError),
    #[error("Failed to get chain name")]
    FailedToGetChainName,
    #[error("Failed to get STARK metadata: {0}")]
    FailedToGetStarkMetadata(String),
}

#[derive(Clone)]
/// Cc3 client that is configured with an url and keypair
/// Must connect to a node that has rpc and websocket enabled
/// - `url`: Creditcoin3 url (rpc + websocket enabled)
/// - `keypair`: Creditcoin3 keypair
pub struct Client {
    pair: sr25519::Pair,
    signing_keypair: Keypair,
    rpc: ReconnectionRpcClient,
    url: String,
}

impl Client {
    /// Create a new instance of cc3 client
    /// - `url`: rpc url of a creditcoin node
    /// - `key`: secret phrase for a creditcoin key
    pub async fn new(url: impl Into<String> + Clone, key: &str) -> Result<Self> {
        let secret_uri = SecretUri::from_str(key)?;
        let signing_keypair = Keypair::from_uri(&secret_uri)?;

        let pair = sr25519::Pair::from_string(key, None)?;

        // Create a new client with with a reconnecting RPC client.
        let rpc = ReconnectionRpcClient::builder()
            // Reconnect with exponential backoff
            //
            // This API is "iterator-like" and we use `take` to limit the number of retries.
            .retry_policy(
                ExponentialBackoff::from_millis(100)
                    .max_delay(Duration::from_secs(10))
                    .take(3),
            )
            // There are other configurations as well that can be found at [`reconnecting_rpc_client::ClientBuilder`].
            .build(url.clone().into().clone())
            .await?;

        Ok(Self {
            pair,
            signing_keypair,
            rpc,
            url: url.into(),
        })
    }

    /// Create a new read-only instance of cc3 client that doesn't require a keypair.
    /// This is useful for read-only operations where signing is not needed.
    /// Uses a dummy keypair internally (which won't be used for read operations).
    /// - `url`: rpc url of a creditcoin node
    pub async fn new_read_only(url: impl Into<String> + Clone) -> Result<Self> {
        // Use a dummy key for read-only operations - it won't be used for signing
        const DUMMY_KEY: &str = "//Alice";
        Self::new(url, DUMMY_KEY).await
    }

    pub async fn api(&self) -> Result<OnlineClient<SubstrateConfig>, Error> {
        Ok(OnlineClient::<SubstrateConfig>::from_rpc_client(self.rpc.clone()).await?)
    }

    #[must_use]
    pub fn sign(&self, message: &[u8]) -> Signature {
        self.signing_keypair.sign(message)
    }

    pub async fn get_chain_key(&self, chain_id: u64, name: Vec<u8>) -> Result<Option<ChainKey>> {
        let chain_key = self
            .api()
            .await?
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

    pub async fn get_chain_name(&self) -> Result<String, Error> {
        let chain_spec = fetch_chainspec_from_rpc_node(self.url.as_str())
            .await
            .map_err(|e| {
                error!("Error fetching chain spec from node: {:?}", e);
                Error::FailedToGetChainName
            })?;
        let json_bytes: Vec<u8> = chain_spec.get().as_bytes().to_vec();

        let spec: GenericChainSpec = GenericChainSpec::from_json_bytes(json_bytes)
            .map_err(|_| Error::FailedToGetChainName)?;

        Ok(spec.id().to_string())
    }

    pub async fn get_supported_chain(&self, chain_key: ChainKey) -> Result<Option<SupportedChain>> {
        let address = cc3::storage()
            .supported_chains()
            .supported_chains(chain_key);

        let result = self
            .api()
            .await?
            .storage()
            .at_latest()
            .await?
            .fetch(&address)
            .await?;

        Ok(result.map(Into::into))
    }

    pub async fn get_supported_chains(&self) -> Result<Vec<SupportedChain>> {
        let mut supported_chains: Vec<SupportedChain> = Vec::new();
        let address = cc3::storage().supported_chains().supported_chains_iter();

        let mut iter = self
            .api()
            .await?
            .storage()
            .at_latest()
            .await?
            .iter(address)
            .await?;

        while let Some(Ok(kv)) = iter.next().await {
            supported_chains.push(kv.value.into());
        }

        Ok(supported_chains)
    }

    /// Fetches the babe randomness from 2 epochs ago
    /// Returns the random a time + the current block number (where it was calculated from)
    pub async fn fetch_babe_randomness_two_epoch_ego(&self) -> Result<(Randomness, u64), Error> {
        let epoch_index = self
            .api()
            .await?
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
            .api()
            .await?
            .storage()
            .at_latest()
            .await?
            .fetch(
                &cc3::storage()
                    .randomness()
                    .randomness_by_epoch_index(two_epoch_ago),
            )
            .await?
            .ok_or(Error::FailedToGetBabeVrf(two_epoch_ago))?;

        Ok((randomness, two_epoch_ago))
    }

    pub async fn get_current_epoch(&self) -> Result<u64, Error> {
        let epoch_index = self
            .api()
            .await?
            .storage()
            .at_latest()
            .await?
            .fetch(&cc3::storage().babe().epoch_index())
            .await?;

        Ok(epoch_index.unwrap_or_default())
    }

    pub async fn target_sample_size(&self, chain_key: u64) -> Result<u32, Error> {
        let storage_query = cc3::storage().attestation().target_sample_size(chain_key);

        let result = self
            .api()
            .await?
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
            .api()
            .await?
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        Ok(result.map(|(_, d)| Digest::from(d.0)))
    }

    /// Check the clients membership in the attestor pallet
    pub async fn check_attestors_membership(&self, chain_key: u64) -> Result<bool> {
        let storage_query = cc3::storage().attestation().active_attestors(chain_key);

        let result = self
            .api()
            .await?
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

    /// Check if the attestor is registered (has a public key)
    /// note: this function early exits if the attestor is not registered
    pub async fn check_attestor_key_is_registered(&self, chain_key: u64) -> Result<bool> {
        let storage_query = cc3::storage()
            .attestation()
            .attestors(chain_key, AccountId32(self.signing_keypair.public_key().0));

        let result = self
            .api()
            .await?
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        match result {
            Some(attestor) => Ok(attestor.bls_public_key.is_some()),
            None => Err(anyhow::anyhow!(
                "Attestor not found in storage, register the attestor first and retry later"
            )),
        }
    }

    /// Check the attestor status
    pub async fn get_attestor_status(&self, chain_key: u64) -> Result<Option<AttestorStatus>> {
        let storage_query = cc3::storage()
            .attestation()
            .attestors(chain_key, AccountId32(self.signing_keypair.public_key().0));

        let result = self
            .api()
            .await?
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        match result {
            Some(attestor) => match attestor.status {
                _ if format!("{:?}", attestor.status) == "Active" => {
                    Ok(Some(AttestorStatus::Active))
                }
                _ if format!("{:?}", attestor.status) == "Idle" => Ok(Some(AttestorStatus::Idle)),
                _ if format!("{:?}", attestor.status) == "Waiting" => {
                    Ok(Some(AttestorStatus::Waiting))
                }
                _ => Ok(None),
            },
            None => Ok(None),
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
            .api()
            .await?
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
            .api()
            .await?
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
        header_number: u64,
        randomness: Randomness,
        epoch_index: u64,
    ) -> Result<ProofOfInclusion, Error> {
        // Get committee set size
        let target_sample_size = self.target_sample_size(chain_key).await?;

        // Get attestor working set size
        let committee_set_size = self.get_attestor_active_set_size(chain_key).await?;

        info!(
            "Target set size: {}, committee set size: {}",
            target_sample_size, committee_set_size
        );

        let proof_of_inclusion = make_proof_of_inclusion(
            committee_set_size as u64,
            u64::from(target_sample_size),
            &randomness,
            &self.pair,
            &self.get_attestor_id(),
            header_number,
            epoch_index,
        )?;

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
            .api()
            .await?
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        Ok(result)
    }

    pub async fn chain_checkpoint_interval(
        &self,
        chain_key: ChainKey,
    ) -> Result<Option<u32>, Error> {
        let storage_query = cc3::storage()
            .attestation()
            .attestation_checkpoint_interval(chain_key);

        let result = self
            .api()
            .await?
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
        let storage_query = cc3::storage()
            .attestation()
            .attestations(chain_key, subxt::utils::H256::from(digest.0));

        let result = self
            .api()
            .await?
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
    ) -> Result<Option<SignedAttestation<Digest, AccountId32>>, Error> {
        let storage_query = cc3::storage()
            .attestation()
            .attestations(chain_key, subxt::utils::H256::from(digest.0));

        let result = self
            .api()
            .await?
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        Ok(result.map(Into::into))
    }

    pub async fn get_last_checkpoint(
        &self,
        chain_key: ChainKey,
    ) -> Result<Option<AttestationCheckpoint>> {
        let storage_query = cc3::storage().attestation().last_checkpoint(chain_key);

        Ok(self
            .api()
            .await?
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?
            .map(|checkpoint| AttestationCheckpoint {
                block_number: checkpoint.block_number,
                digest: Digest::from_slice(&checkpoint.digest.0),
            }))
    }

    pub async fn get_checkpoint_by_height(
        &self,
        chain_key: ChainKey,
        block_number: u64,
    ) -> Result<Option<AttestationCheckpoint>> {
        let storage_query = cc3::storage()
            .attestation()
            .checkpoints(chain_key, block_number);

        Ok(self
            .api()
            .await?
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?
            .map(|digest| AttestationCheckpoint {
                block_number,
                digest: Digest::from_slice(&digest.0),
            }))
    }

    pub async fn get_attestations_for_chain(
        &self,
        chain_key: ChainKey,
    ) -> Result<Vec<SignedAttestation<Digest, AccountId32>>> {
        let mut attestations: Vec<SignedAttestation<Digest, AccountId32>> = Vec::new();

        // Address to the root of a storage entry that we'd like to iterate over
        // concatenated with the encoded first key to the Attestations double map,
        // a ChainKey
        let address = cc3::storage().attestation().attestations_iter1(chain_key);

        let mut iter = self
            .api()
            .await?
            .storage()
            .at_latest()
            .await?
            .iter(address)
            .await?;

        while let Some(Ok(kv)) = iter.next().await {
            attestations.push(kv.value.into());
        }

        attestations.sort_by(
            |a: &SignedAttestation<Digest, AccountId32>,
             b: &SignedAttestation<Digest, AccountId32>| {
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

        let mut iter = self
            .api()
            .await?
            .storage()
            .at_latest()
            .await?
            .iter(address)
            .await?;

        while let Some(Ok(kv)) = iter.next().await {
            if kv.key_bytes.len() < 8 {
                error!(
                    "Storage key for chainkey {} is less than 8 bytes, checkpoint: {:?}",
                    chain_key, kv
                );
                continue;
            }
            let last_8: Result<[u8; 8], _> = kv.key_bytes[kv.key_bytes.len() - 8..].try_into();
            if let Ok(block_number_byes) = last_8 {
                let block_number = u64::from_be_bytes(block_number_byes);
                let checkpoint = AttestationCheckpoint {
                    block_number,
                    digest: sp_core::H256::from(kv.value.0),
                };
                checkpoints.push(checkpoint);
            } else {
                error!(
                    "Failed to get last 8 bytes of storage key for chainkey {}, checkpoint: {:?}",
                    chain_key, kv
                );
            }
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
        let mut params = RpcParams::new();
        params
            .push(attestation)
            .map_err(|_| Error::FailedToSubmit)?;

        let r = subxt::backend::rpc::RpcClient::from(self.rpc.clone());

        match r.request::<()>("attestor_submitAttestation", params).await {
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
            .transfer_allow_death(subxt::utils::MultiAddress::Id(target), amount.into());

        let params = if let Some(account_nonce) = account_nonce {
            DefaultExtrinsicParamsBuilder::new()
                .nonce(account_nonce)
                .build()
        } else {
            DefaultExtrinsicParamsBuilder::new().build()
        };

        let ext = self
            .api()
            .await?
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
            .api()
            .await?
            .tx()
            .account_nonce(&AccountId32(self.signing_keypair.public_key().0))
            .await?;

        Ok(nonce)
    }

    pub async fn get_attestor_active_set_size(&self, chain_key: u64) -> Result<usize, Error> {
        let storage_query = cc3::storage().attestation().active_attestors(chain_key);

        let result = self
            .api()
            .await?
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        match result {
            Some(result) => Ok(result.len()),
            None => Ok(0),
        }
    }

    pub async fn set_attestation_chain_genesis_block_number(
        &self,
        account_nonce: Option<u64>,
        chain_key: ChainKey,
        genesis_block_number: u64,
    ) -> Result<(), Error> {
        let call = cc3::runtime_types::creditcoin3_runtime::RuntimeCall::Attestation(
            cc3::runtime_types::pallet_attestation_poc::pallet::Call::set_attestation_chain_genesis_block_number { chain_key, genesis_block_number }
        );

        let tx = cc3::tx().sudo().sudo(call);

        let params = if let Some(account_nonce) = account_nonce {
            DefaultExtrinsicParamsBuilder::new()
                .nonce(account_nonce)
                .build()
        } else {
            DefaultExtrinsicParamsBuilder::new().build()
        };

        let ext = self
            .api()
            .await?
            .tx()
            .create_signed(&tx, &self.signing_keypair, params)
            .await?
            .submit_and_watch()
            .await?;

        let hash = ext.extrinsic_hash();
        debug!(
            "Set attestation chain genesis block number extrinsic submitted with hash: {:?}",
            hash
        );

        Ok(())
    }

    pub async fn get_attestation_chain_genesis_block_number(
        &self,
        chain_key: ChainKey,
    ) -> Result<u64, Error> {
        let storage_query = cc3::storage()
            .attestation()
            .attestation_chain_genesis_block_number(chain_key);

        let result = self
            .api()
            .await?
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        Ok(result.unwrap_or_default())
    }

    pub async fn get_attestation_vote_acceptance_window(
        &self,
        chain_key: ChainKey,
    ) -> Result<Option<u64>, Error> {
        let storage_query = cc3::storage()
            .attestation()
            .vote_acceptance_window(chain_key);

        let result = self
            .api()
            .await?
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        Ok(result)
    }
}

impl<A> From<CcSignedAttestation<H256, A>> for SignedAttestation<Digest, A> {
    fn from(attestation: CcSignedAttestation<H256, A>) -> Self {
        SignedAttestation {
            attestation: attestation.attestation.into(),
            signature: attestation.signature,
            attestors: attestation.attestors,
            continuity_proof: attestation.continuity_proof.into(),
        }
    }
}

impl From<CcAttestationFragment> for AttestationFragmentSerializable {
    fn from(fragment: CcAttestationFragment) -> Self {
        let fragment =
            AttestationFragment::from_blocks(fragment.blocks.into_iter().map(Into::into).collect());
        (&fragment).into()
    }
}

impl From<CcBlockSerializable> for Block {
    fn from(block: CcBlockSerializable) -> Self {
        Block {
            block_number: block.block_number,
            root: sp_core::H256::from_slice(&block.root.0),
            prev_digest: sp_core::H256::from_slice(&block.prev_digest.0),
            digest: sp_core::H256::from_slice(&block.digest.0),
        }
    }
}

impl From<CcAttestation<H256>> for Attestation<Digest> {
    fn from(attestation: CcAttestation<H256>) -> Self {
        Attestation {
            chain_key: attestation.chain_key,
            header_number: attestation.header_number,
            header_hash: sp_core::H256::from(attestation.header_hash.0),
            root: sp_core::H256::from(attestation.root.0),
            prev_digest: attestation
                .prev_digest
                .map(|digest| sp_core::H256::from(digest.0)),
        }
    }
}

impl From<CcAttestationCheckpoint> for AttestationCheckpoint {
    fn from(checkpoint: CcAttestationCheckpoint) -> Self {
        AttestationCheckpoint {
            block_number: checkpoint.block_number,
            digest: sp_core::H256::from(checkpoint.digest.0),
        }
    }
}

impl From<CcSupportedChain> for SupportedChain {
    fn from(chain: CcSupportedChain) -> Self {
        SupportedChain {
            chain_id: chain.chain_id,
            chain_name: chain.chain_name,
            chain_encoding: ChainEncodingVersion::from(chain.chain_encoding),
            maturity_strategy: chain.maturity_strategy,
        }
    }
}

impl From<CcChainEncodingVersion> for ChainEncodingVersion {
    fn from(version: CcChainEncodingVersion) -> Self {
        match version {
            CcChainEncodingVersion::V1 => ChainEncodingVersion::V1,
        }
    }
}

#[cfg(feature = "std")]
impl From<CcChainEncodingVersion> for ccnext_abi_encoding::abi::EncodingVersion {
    fn from(version: CcChainEncodingVersion) -> Self {
        match version {
            CcChainEncodingVersion::V1 => ccnext_abi_encoding::abi::EncodingVersion::V1,
        }
    }
}
