use std::sync::Arc;

use arc_swap::ArcSwap;
use serde::Serialize;
use sp_core::U256;
pub use subxt::utils::{AccountId32, H256};
use subxt::{
    backend::{
        legacy::LegacyRpcMethods,
        rpc::{RpcClient, RpcParams},
    },
    config::DefaultExtrinsicParamsBuilder,
    error::RpcError,
    OnlineClient, SubstrateConfig,
};
use subxt_signer::sr25519::Signature;
use thiserror::Error;
use tracing::{debug, error, info, warn};

use cc3::runtime_types::{
    attestor_primitives::{
        block::ContinuityProof as CcContinuityProof,
        AttestationCheckpoint as CcAttestationCheckpoint, AttestationData as CcAttestationData,
        ChainEncodingVersion as CcChainEncodingVersion, SignedAttestation as CcSignedAttestation,
    },
    supported_chains_primitives::SupportedChain as CcSupportedChain,
};

use attestor_primitives::{
    block::ContinuityProof, Attestation as RpcAttestation, AttestationCheckpoint, AttestationData,
    AttestorId, AttestorStatus, BlsPublicKey, BlsSignature, ChainEncodingVersion, ChainKey, Digest,
    SignedAttestation,
};
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

pub mod api;
pub mod attestation;
pub mod signer;

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
    #[error("Subxt error: {0:?}")]
    SubxtError(#[from] subxt::Error),
    #[error("Rpc error: {0:?}")]
    RpcError(#[from] RpcError),
    #[error("Invalid rpc url")]
    InvalidUrl,
    #[error("Failed to create proof of inclusion: {0}")]
    FailedToCreateProofOfInclusion(#[from] VrfError),
    #[error("Failed to get STARK metadata: {0}")]
    FailedToGetStarkMetadata(String),
    #[error("Attestor not found in storage, register the attestor first and retry later")]
    NotRegistered,
    #[error("Caller cannot pay fees for the transaction")]
    CallerCannotPayFees,
    #[error("Caller doesn't have sufficient funds to execute the transaction: {0:?}")]
    CallerDoesntHaveSufficientFunds(#[from] subxt::error::TokenError),
    #[error("Transaction timed out waiting for finalization")]
    TransactionTimeout,
}

/// Inner mutable state of [`Client`]: the live subxt RPC handle and its
/// derived `OnlineClient` / `LegacyRpcMethods`. Stored behind
/// [`ArcSwap`] inside [`Client`] so that a successful [`Client::reconnect`]
/// atomically replaces the live connection for *every* `Arc<Client>`
/// holder at once.
struct ClientInner {
    rpc: RpcClient,
    api: OnlineClient<SubstrateConfig>,
    legacy: LegacyRpcMethods<SubstrateConfig>,
    updater_handle: tokio::task::JoinHandle<()>,
}

// Manual Drop implementation to ensure that the updater task is aborted when the client is dropped.
impl Drop for ClientInner {
    fn drop(&mut self) {
        self.updater_handle.abort();
    }
}

// Timeout for the runtime metadata updater task to wait before retrying after an error or stream end.
const UPDATER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Cc3 client that is configured with an url and keypair
/// Must connect to a node that has rpc and websocket enabled
/// - `url`: Creditcoin3 url (rpc + websocket enabled)
/// - `keypair`: Creditcoin3 keypair
///
/// The live subxt RPC connection lives behind an [`ArcSwap`] so that a single
/// `Arc<Client>` shared across many tasks can be reconnected in place: once
/// any holder calls [`Client::reconnect`], every other holder picks up the
/// fresh subxt `RpcClient` / `OnlineClient` on its next call. The previous
/// design swapped fields on a value-cloned `Client`, which left other
/// `Arc<Client>` holders pinned to a stale (and often `RestartNeeded`)
/// connection until the process was restarted.
pub struct Client {
    signer: signer::CC3Signer,
    url: String,
    inner: ArcSwap<ClientInner>,
}

impl Clone for Client {
    /// Clone the client by snapshotting the *current* live connection into a
    /// fresh `ArcSwap`. Note that the resulting `Client` has its own
    /// `ArcSwap`: subsequent `reconnect()` calls on either side will not be
    /// observed by the other. For shared-reconnect semantics, share an
    /// `Arc<Client>` (every holder of the same `Arc<Client>` *does* see
    /// reconnects atomically).
    fn clone(&self) -> Self {
        Self {
            signer: self.signer.clone(),
            url: self.url.clone(),
            inner: ArcSwap::new(self.inner.load_full()),
        }
    }
}

#[allow(clippy::missing_fields_in_debug)]
impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").field("url", &self.url).finish()
    }
}

impl Client {
    /// Create a new instance of cc3 client
    /// - `url`: rpc url of a creditcoin node
    /// - `key`: secret phrase for a creditcoin key
    pub async fn new(url: impl Into<String> + Clone, key: &str) -> anyhow::Result<Self> {
        let signer = signer::CC3Signer::new(key)?;
        let url: String = url.into();
        let inner = Self::build_inner(&url).await?;

        Ok(Self {
            signer,
            url,
            inner: ArcSwap::new(Arc::new(inner)),
        })
    }

    /// Open a fresh subxt connection (RPC + `OnlineClient` + `LegacyRpcMethods`)
    /// against `url`. Used by both [`Client::new`] and [`Client::reconnect`].
    async fn build_inner(url: &str) -> Result<ClientInner, Error> {
        let rpc = RpcClient::from_insecure_url(url.to_owned()).await?;
        let api = OnlineClient::<SubstrateConfig>::from_rpc_client(rpc.clone()).await?;
        let legacy = LegacyRpcMethods::<SubstrateConfig>::new(rpc.clone());

        // In order to keep metadata up to date, we need to continuously poll the node for updates.
        // This is done by spawning a background task that calls `perform_runtime_updates` in a loop.
        // If the stream ends or an error occurs, we log the error and retry after a short delay.
        let updater = api.updater();
        let updater_handle = tokio::spawn(async move {
            loop {
                match updater.perform_runtime_updates().await {
                    Ok(()) => info!(
                        "runtime metadata updater stream ended, retrying in {}s",
                        UPDATER_TIMEOUT.as_secs()
                    ),
                    Err(e) => warn!(
                        "runtime metadata updater stopped: {e}, retrying in {}s",
                        UPDATER_TIMEOUT.as_secs()
                    ),
                }

                tokio::time::sleep(UPDATER_TIMEOUT).await;
            }
        });

        Ok(ClientInner {
            rpc,
            api,
            legacy,
            updater_handle,
        })
    }

    /// Atomically replace the live subxt connection with a freshly-opened one.
    ///
    /// Takes `&self` (not `&mut self`) on purpose: a single `Arc<Client>`
    /// shared across many tasks (e.g. proof-gen-api-server's events task and
    /// every continuity builder) can call this without coordination, and all
    /// callers immediately observe the new connection on their next
    /// `api()`/`legacy()`/`rpc()` access.
    pub async fn reconnect(&self) -> Result<(), Error> {
        let inner = Self::build_inner(&self.url).await?;
        self.inner.store(Arc::new(inner));
        Ok(())
    }

    /// Create a new read-only instance of cc3 client that doesn't require a keypair.
    /// This is useful for read-only operations where signing is not needed.
    /// Uses a dummy keypair internally (which won't be used for read operations).
    /// - `url`: rpc url of a creditcoin node
    pub async fn new_read_only(url: impl Into<String> + Clone) -> anyhow::Result<Self> {
        // Use a dummy key for read-only operations - it won't be used for signing
        const DUMMY_KEY: &str = "//Alice";
        Self::new(url, DUMMY_KEY).await
    }

    /// Snapshot of the live subxt `OnlineClient`.
    ///
    /// The returned value is a cheap arc-internal clone of the current
    /// connection at call time. After a `reconnect()`, subsequent `api()`
    /// calls return the fresh client; previously-snapshotted clients keep
    /// using the old connection until they're dropped.
    #[must_use]
    pub fn api(&self) -> OnlineClient<SubstrateConfig> {
        self.inner.load().api.clone()
    }

    /// Snapshot of the live subxt `LegacyRpcMethods`. See [`Client::api`] for
    /// snapshot semantics.
    #[must_use]
    pub fn legacy(&self) -> LegacyRpcMethods<SubstrateConfig> {
        self.inner.load().legacy.clone()
    }

    /// Snapshot of the live subxt `RpcClient`. Used internally for raw RPC
    /// requests such as attestation submission.
    fn rpc(&self) -> RpcClient {
        self.inner.load().rpc.clone()
    }

    #[must_use]
    pub fn runtime_api() -> cc3::runtime_apis::RuntimeApi {
        cc3::apis()
    }

    #[must_use]
    pub fn sign(&self, message: &[u8]) -> Signature {
        self.signer.sign(message)
    }

    pub async fn get_chain_key(
        &self,
        chain_id: u64,
        name: Vec<u8>,
    ) -> Result<Option<ChainKey>, Error> {
        let chain_key = self
            .api()
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

    pub async fn get_supported_chain(
        &self,
        chain_key: ChainKey,
    ) -> Result<Option<SupportedChain>, Error> {
        let address = cc3::storage()
            .supported_chains()
            .supported_chains(chain_key);

        let result = self
            .api()
            .storage()
            .at_latest()
            .await?
            .fetch(&address)
            .await?;

        Ok(result.map(Into::into))
    }

    pub async fn get_supported_chains(&self) -> Result<Vec<SupportedChain>, Error> {
        let mut supported_chains: Vec<SupportedChain> = Vec::new();
        let address = cc3::storage().supported_chains().supported_chains_iter();

        let mut iter = self
            .api()
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
    pub async fn fetch_babe_randomness_two_epoch_ago(&self) -> Result<(Randomness, u64), Error> {
        let epoch_index = self
            .api()
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
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        Ok(result.map(|(_, d)| Digest::from(d.0)))
    }

    /// Check the clients membership in the attestor pallet
    pub async fn check_attestors_membership(&self, chain_key: u64) -> Result<bool, Error> {
        let storage_query = cc3::storage().attestation().active_attestors(chain_key);

        let result = self
            .api()
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        match result {
            Some(result) => Ok(result.contains(&self.signer.account_id())),
            None => Ok(false),
        }
    }

    /// Check if the attestor is registered (has a public key)
    /// note: this function early exits if the attestor is not registered
    pub async fn check_attestor_key_is_registered(&self, chain_key: u64) -> Result<bool, Error> {
        let storage_query = cc3::storage()
            .attestation()
            .attestors(chain_key, self.signer.account_id());

        let result = self
            .api()
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        match result {
            Some(attestor) => Ok(attestor.bls_public_key.is_some()),
            None => Err(Error::NotRegistered),
        }
    }

    /// Check the attestor status
    pub async fn get_attestor_status(
        &self,
        chain_key: u64,
    ) -> Result<Option<AttestorStatus>, Error> {
        let storage_query = cc3::storage()
            .attestation()
            .attestors(chain_key, self.signer.account_id());

        let result = self
            .api()
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
                _ if format!("{:?}", attestor.status) == "Leaving" => {
                    Ok(Some(AttestorStatus::Leaving))
                }
                _ => Ok(None),
            },
            None => Ok(None),
        }
    }

    /// Register to the attestation pallet
    pub async fn attestor_register(
        &self,
        chain_key: u64,
        attestor_id: AccountId32,
        account_nonce: Option<u64>,
    ) -> Result<(), Error> {
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

        let tx_progress = self
            .api()
            .tx()
            .create_signed(&tx, &self.signer.signing_keypair, params)
            .await?
            .submit_and_watch()
            .await
            .map_err(|e| {
                if utils::is_fee_error(&e) {
                    Error::CallerCannotPayFees
                } else {
                    e.into()
                }
            })?;

        utils::handle_tx(tx_progress, "Register Attestor").await
    }

    pub async fn attestor_chill(
        &self,
        chain_key: u64,
        attestor_id: AccountId32,
        account_nonce: Option<u64>,
    ) -> Result<(), Error> {
        let tx = cc3::tx().attestation().chill(chain_key, attestor_id);

        let params = if let Some(account_nonce) = account_nonce {
            DefaultExtrinsicParamsBuilder::new()
                .nonce(account_nonce)
                .build()
        } else {
            DefaultExtrinsicParamsBuilder::new().build()
        };

        let tx_progress = self
            .api()
            .tx()
            .create_signed(&tx, &self.signer.signing_keypair, params)
            .await?
            .submit_and_watch()
            .await
            .map_err(|e| {
                if utils::is_fee_error(&e) {
                    Error::CallerCannotPayFees
                } else {
                    e.into()
                }
            })?;

        utils::handle_tx(tx_progress, "Chill Attestor").await
    }

    pub async fn attestor_unregister(
        &self,
        chain_key: u64,
        attestor_id: AccountId32,
        account_nonce: Option<u64>,
    ) -> Result<(), Error> {
        let tx = cc3::tx()
            .attestation()
            .unregister_attestor(chain_key, attestor_id);

        let params = if let Some(account_nonce) = account_nonce {
            DefaultExtrinsicParamsBuilder::new()
                .nonce(account_nonce)
                .build()
        } else {
            DefaultExtrinsicParamsBuilder::new().build()
        };

        let tx_progress = self
            .api()
            .tx()
            .create_signed(&tx, &self.signer.signing_keypair, params)
            .await?
            .submit_and_watch()
            .await
            .map_err(|e| {
                if utils::is_fee_error(&e) {
                    Error::CallerCannotPayFees
                } else {
                    e.into()
                }
            })?;

        utils::handle_tx(tx_progress, "Unregister Attestor").await
    }

    pub async fn start_attesting(
        &self,
        chain_key: ChainKey,
        bls_public_key: BlsPublicKey,
        proof_of_possession: BlsSignature,
    ) -> Result<(), Error> {
        let tx = cc3::tx()
            .attestation()
            .attest(chain_key, bls_public_key, proof_of_possession);

        let tx_progress = self
            .api()
            .tx()
            .sign_and_submit_then_watch_default(&tx, &self.signer.signing_keypair)
            .await
            .map_err(|e| {
                if utils::is_fee_error(&e) {
                    Error::CallerCannotPayFees
                } else {
                    e.into()
                }
            })?;

        utils::handle_tx(tx_progress, "Start Attesting").await
    }

    /// `sign_babe_vrf` signs babe's author vrf randomness with the configured key and returns the output as integer
    /// the method extracts the S component bytes from the signature. The bytes of the S component are converted into a u64 integer using little-endian byte order.
    pub async fn sign_vrf_production(
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

        info!("Target set size: {target_sample_size}, committee set size: {committee_set_size}",);

        let proof_of_inclusion = make_proof_of_inclusion(
            committee_set_size as u64,
            u64::from(target_sample_size),
            &randomness,
            &self.signer.pair,
            &self.attestor_id(),
            header_number,
            epoch_index,
        )?;

        Ok(proof_of_inclusion)
    }

    pub async fn sign_vrf_submission(
        &self,
        chain_key: ChainKey,
        header_number: u64,
        randomness: Randomness,
        epoch_index: u64,
    ) -> Result<ProofOfInclusion, Error> {
        // Get committee set size
        let target_sample_size = 3;

        // Get attestor working set size
        let committee_set_size = self.get_attestor_active_set_size(chain_key).await?;

        info!("committee set size: {committee_set_size}",);

        let proof_of_inclusion = make_proof_of_inclusion(
            committee_set_size as u64,
            target_sample_size,
            &randomness,
            &self.signer.pair,
            &self.attestor_id(),
            header_number,
            epoch_index,
        )?;

        Ok(proof_of_inclusion)
    }

    #[must_use]
    pub fn attestor_id(&self) -> AttestorId {
        self.signer.attestor_id()
    }

    pub async fn chain_attestation_interval(
        &self,
        chain_key: ChainKey,
    ) -> Result<Option<u64>, Error> {
        let storage_query = cc3::storage()
            .attestation()
            .chain_attestation_interval(chain_key);

        let result = self
            .api()
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
    ) -> Result<Option<u64>, Error> {
        let storage_query = cc3::storage()
            .attestation()
            .attestation_checkpoint_interval(chain_key);

        let result = self
            .api()
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?
            .map(Into::into);

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
    ) -> Result<Option<AttestationCheckpoint>, Error> {
        let storage_query = cc3::storage().attestation().last_checkpoint(chain_key);

        Ok(self
            .api()
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
    ) -> Result<Option<AttestationCheckpoint>, Error> {
        let storage_query = cc3::storage()
            .attestation()
            .checkpoints(chain_key, block_number);

        Ok(self
            .api()
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
    ) -> Result<Vec<SignedAttestation<Digest, AccountId32>>, Error> {
        let mut attestations: Vec<SignedAttestation<Digest, AccountId32>> = Vec::new();

        // Address to the root of a storage entry that we'd like to iterate over
        // concatenated with the encoded first key to the Attestations double map,
        // a ChainKey
        let address = cc3::storage().attestation().attestations_iter1(chain_key);

        let mut iter = self
            .api()
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
    ) -> Result<Vec<AttestationCheckpoint>, Error> {
        let mut checkpoints = Vec::new();

        // Address to the root of a storage entry that we'd like to iterate over
        // concatenated with the encoded first key to the Checkpoints double map,
        // a ChainKey.
        let address = cc3::storage().attestation().checkpoints_iter1(chain_key);

        let mut iter = self
            .api()
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
            if let Ok(block_number_bytes) = last_8 {
                // Substrate encodes u64 as little-endian when using Identity hasher
                let block_number = u64::from_le_bytes(block_number_bytes);
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

        match self
            .rpc()
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
        amount: u128,
        account_nonce: Option<u64>,
    ) -> Result<(), Error> {
        let tx = cc3::tx()
            .balances()
            .transfer_allow_death(subxt::utils::MultiAddress::Id(target), amount);

        let params = if let Some(account_nonce) = account_nonce {
            DefaultExtrinsicParamsBuilder::new()
                .nonce(account_nonce)
                .build()
        } else {
            DefaultExtrinsicParamsBuilder::new().build()
        };

        let tx_progress = self
            .api()
            .tx()
            .create_signed(&tx, &self.signer.signing_keypair, params)
            .await?
            .submit_and_watch()
            .await
            .map_err(|e| {
                if utils::is_fee_error(&e) {
                    Error::CallerCannotPayFees
                } else {
                    e.into()
                }
            })?;

        utils::handle_tx(tx_progress, "Transfer").await
    }

    pub async fn set_balance(
        &self,
        target: AccountId32,
        amount: u128,
        account_nonce: Option<u64>,
    ) -> Result<(), Error> {
        let tx = cc3::tx().sudo().sudo(cc3::Call::Balances(
            cc3::balances::Call::force_set_balance {
                who: subxt::utils::MultiAddress::Id(target),
                new_free: amount,
            },
        ));

        let params = if let Some(account_nonce) = account_nonce {
            DefaultExtrinsicParamsBuilder::new()
                .nonce(account_nonce)
                .build()
        } else {
            DefaultExtrinsicParamsBuilder::new().build()
        };

        let tx_progress = self
            .api()
            .tx()
            .create_signed(&tx, &self.signer.signing_keypair, params)
            .await?
            .submit_and_watch()
            .await
            .map_err(|e| {
                if utils::is_fee_error(&e) {
                    Error::CallerCannotPayFees
                } else {
                    e.into()
                }
            })?;

        utils::handle_tx(tx_progress, "Set Balance").await
    }

    pub async fn get_free_balance(&self, account: &AccountId32) -> Result<u128, Error> {
        let storage_query = cc3::storage().system().account(account);
        let account_info = self
            .api()
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;
        Ok(account_info.map_or(0, |info| info.data.free))
    }

    pub async fn get_account_nonce(&self) -> Result<u64, Error> {
        let nonce = self
            .api()
            .tx()
            .account_nonce(&self.signer.account_id())
            .await?;

        Ok(nonce)
    }

    pub async fn get_attestor_active_set(&self, chain_key: u64) -> Result<Vec<AccountId32>, Error> {
        let storage_query = cc3::storage().attestation().active_attestors(chain_key);

        let result = self
            .api()
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        Ok(result.unwrap_or_default())
    }

    pub async fn get_attestor_active_set_size(&self, chain_key: u64) -> Result<usize, Error> {
        Ok(self.get_attestor_active_set(chain_key).await?.len())
    }

    pub async fn set_attestation_chain_genesis_block_number(
        &self,
        account_nonce: Option<u64>,
        chain_key: ChainKey,
        genesis_block_number: u64,
    ) -> Result<(), Error> {
        let call = cc3::runtime_types::creditcoin3_runtime::RuntimeCall::Attestation(
            cc3::runtime_types::pallet_attestation::pallet::Call::set_attestation_chain_genesis_block_number { chain_key, genesis_block_number }
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
            .tx()
            .create_signed(&tx, &self.signer.signing_keypair, params)
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
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        Ok(result.unwrap_or_default())
    }
}

mod utils {
    /// Timeout for waiting on extrinsic finalization.
    /// Set to 120 seconds which is around 8 blocks on a 15 second block time.
    const FINALIZATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

    /// This is a fallback error message that we can use to detect insufficient funds errors in the absence of a more structured error from the RPC layer.
    /// Sourced from: <https://github.com/paritytech/polkadot-sdk/blob/06bded7ab7ac6a50e0aeba48c0f7f5ca548c3573/substrate/primitives/runtime/src/transaction_validity.rs#L116>
    const INABILITY_TO_PAY_SOME_FEE_MSG: &str = "Inability to pay some fee";

    pub(super) async fn handle_tx(
        tx: subxt::tx::TxProgress<
            subxt::SubstrateConfig,
            subxt::OnlineClient<subxt::SubstrateConfig>,
        >,
        msg: &str,
    ) -> Result<(), crate::Error> {
        match tokio::time::timeout(FINALIZATION_TIMEOUT, tx.wait_for_finalized_success()).await {
            Ok(Ok(ext)) => {
                let hash = ext.extrinsic_hash();
                tracing::debug!("{} extrinsic succeeded with hash: {:?}", msg, hash);
                Ok(())
            }
            Ok(Err(err)) if is_fee_error(&err) => Err(crate::Error::CallerCannotPayFees),
            Ok(Err(subxt::Error::Runtime(subxt::error::DispatchError::Token(token_error)))) => {
                // If we get a token error, it means the transaction was valid but failed to execute due to insufficient funds or similar issues. We can return a specific error for this case.
                Err(crate::Error::CallerDoesntHaveSufficientFunds(token_error))
            }
            Ok(Err(e)) => {
                // Any other error that occurs while waiting for the transaction to be finalized can be treated as a generic submission failure.
                Err(e.into())
            }
            Err(_) => {
                // Timeout while waiting for the transaction to be finalized. We treat this as a specific timeout error.
                Err(crate::Error::TransactionTimeout)
            }
        }
    }

    pub(super) fn is_fee_error(e: &subxt::Error) -> bool {
        if let subxt::Error::Rpc(subxt::error::RpcError::ClientError(err)) = e {
            if let Some(subxt::ext::jsonrpsee::core::client::Error::Call(call_err)) =
                err.downcast_ref::<subxt::ext::jsonrpsee::core::client::Error>()
            {
                if let Some(data) = call_err.data() {
                    return data.get().contains(INABILITY_TO_PAY_SOME_FEE_MSG);
                }
            }
        }

        false
    }
}

// NOTE: a lot of these type-conversion shenanigans is due to the fact that we use a different type
// of `primitive_types` via `sp_core` than `subxt` exposes. In the future, it would be nice to see
// if we can resolve this dependency mismatch, perhaps by downgrading our version of `subxt`
// (easier) or updating the version of `sp_core` we use (harder).

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

impl From<SignedAttestation<Digest, AttestorId>> for CcSignedAttestation<H256, AccountId32> {
    fn from(attestation: SignedAttestation<Digest, AttestorId>) -> Self {
        CcSignedAttestation {
            attestation: attestation.attestation.into(),
            signature: attestation.signature,
            attestors: attestation
                .attestors
                .iter()
                .map(|att| {
                    let bytes: &[u8] = att.account_id().as_ref();
                    AccountId32(bytes.try_into().unwrap())
                })
                .collect(),
            continuity_proof: attestation.continuity_proof.into(),
        }
    }
}

impl From<CcContinuityProof> for ContinuityProof {
    fn from(p: CcContinuityProof) -> Self {
        Self {
            lower_endpoint_digest: sp_core::H256::from_slice(&p.lower_endpoint_digest.0),
            roots: p
                .roots
                .into_iter()
                .map(|r| sp_core::H256::from_slice(&r.0))
                .collect(),
        }
    }
}

impl From<ContinuityProof> for CcContinuityProof {
    fn from(p: ContinuityProof) -> Self {
        CcContinuityProof {
            lower_endpoint_digest: H256(p.lower_endpoint_digest.0),
            roots: p.roots.into_iter().map(|r| H256(r.0)).collect(),
        }
    }
}

impl From<CcAttestationData<H256>> for AttestationData<Digest> {
    fn from(attestation: CcAttestationData<H256>) -> Self {
        AttestationData {
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

impl From<AttestationData<Digest>> for CcAttestationData<H256> {
    fn from(attestation: AttestationData<Digest>) -> Self {
        CcAttestationData {
            chain_key: attestation.chain_key,
            header_number: attestation.header_number,
            header_hash: H256(attestation.header_hash.0),
            root: H256(attestation.root.0),
            prev_digest: attestation.prev_digest.map(|digest| H256(digest.0)),
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
impl From<CcChainEncodingVersion> for usc_abi_encoding::common::EncodingVersion {
    fn from(version: CcChainEncodingVersion) -> Self {
        match version {
            CcChainEncodingVersion::V1 => usc_abi_encoding::common::EncodingVersion::V1,
        }
    }
}

#[cfg(test)]
mod tests {
    //! Tests that lock in the *shared-Arc reconnect* contract added in
    //! CSUB-2039: a single `Arc<Client>` shared across many tasks must see a
    //! `reconnect()` call atomically refresh the underlying subxt connection
    //! for every holder, not just the caller.
    //!
    //! We can't stand up a real subxt `RpcClient` in a unit test (it needs a
    //! live WS endpoint), so these tests exercise the `ArcSwap<ClientInner>`
    //! plumbing directly. The production `Client::reconnect` is a thin
    //! wrapper that builds a fresh `ClientInner` and calls
    //! `self.inner.store(...)` — if the store/load contract holds for one
    //! Arc holder, it holds for all of them.

    use super::*;

    // We can't construct a real `ClientInner` in a unit test (subxt's
    // `RpcClient`/`OnlineClient`/`LegacyRpcMethods` need a live endpoint),
    // so the tests below exercise the underlying `ArcSwap` swap semantics
    // directly. `Client::reconnect` is a thin wrapper around exactly this
    // pattern: build a fresh inner, then `self.inner.store(Arc::new(inner))`.
    // If the contract holds at the `ArcSwap` layer it holds for `Client`.

    /// Locks in the contract that `ArcSwap::store` is observed atomically by
    /// every holder of the same `ArcSwap`. This is the exact mechanism
    /// `Client::reconnect` uses to refresh the live subxt connection for
    /// every `Arc<Client>` holder, so we test the contract here even though
    /// we can't easily build a real `ClientInner` in a unit test.
    #[test]
    fn arcswap_shared_reconnect_contract() {
        // Stand in for `ClientInner`: any `Arc`-able payload works.
        struct StubInner(#[allow(dead_code)] u64);

        let swap = Arc::new(ArcSwap::new(Arc::new(StubInner(0))));
        let observer_a = Arc::clone(&swap);
        let observer_b = Arc::clone(&swap);

        // Both observers see the same initial inner.
        let initial_a = observer_a.load_full();
        let initial_b = observer_b.load_full();
        assert!(
            Arc::ptr_eq(&initial_a, &initial_b),
            "two Arc<ArcSwap> holders must observe the same initial inner"
        );

        // Simulate `reconnect()`: build a fresh inner and store it via one
        // observer. With the new `&self` reconnect signature, this is the
        // exact code path proof-gen-api-server's events task takes.
        let new_inner = Arc::new(StubInner(1));
        observer_a.store(Arc::clone(&new_inner));

        // Both observers must now see the freshly-stored inner.
        let after_a = observer_a.load_full();
        let after_b = observer_b.load_full();
        assert!(
            Arc::ptr_eq(&after_a, &new_inner),
            "the observer that called store() must see the new inner"
        );
        assert!(
            Arc::ptr_eq(&after_b, &new_inner),
            "the *other* observer must also see the new inner \
             (this is the bug CSUB-2039 fixes)"
        );

        // Old inner is no longer the live one.
        assert!(
            !Arc::ptr_eq(&after_b, &initial_b),
            "after store(), the live inner must differ from the original"
        );
    }

    /// Sanity check that `Client::clone` produces a `Client` with its *own*
    /// `ArcSwap` (i.e. a value-cloned Client *cannot* be reconnected on
    /// behalf of another Client — you have to share via `Arc<Client>` for
    /// the shared-reconnect semantics). This documents the deliberate
    /// distinction between value-clone (independent connections) and
    /// `Arc::clone` (shared connection).
    ///
    /// We can't build a real `Client` in unit tests, so we restate the
    /// contract on `ArcSwap` directly: cloning the *outer* Arc shares the
    /// swap, while replacing the inner `ArcSwap` with a fresh one (as
    /// `Client::clone` does) does not.
    #[test]
    fn arcswap_clone_is_shared_only_when_outer_arc_is_shared() {
        let swap_a = ArcSwap::new(Arc::new(0u64));
        // `Client::clone` snapshots the current inner into a *new* `ArcSwap`.
        let swap_b = ArcSwap::new(swap_a.load_full());

        swap_a.store(Arc::new(42u64));

        assert_eq!(**swap_a.load(), 42, "caller of store() sees the update");
        assert_eq!(
            **swap_b.load(),
            0,
            "a value-cloned Client has its own ArcSwap and is unaffected"
        );
    }
}
