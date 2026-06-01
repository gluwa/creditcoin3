use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use sp_core::U256;
use subxt::{
    backend::rpc::RpcClient, config::DefaultExtrinsicParamsBuilder, error::RpcError,
    ext::jsonrpsee::core::client::Error as JsonRpseeError, OnlineClient, SubstrateConfig,
};
use subxt_signer::sr25519::Signature;
use thiserror::Error;

use cc3::runtime_types::{
    attestor_primitives::{
        block::ContinuityProof as CcContinuityProof,
        AttestationCheckpoint as CcAttestationCheckpoint, AttestationData as CcAttestationData,
        ChainEncodingVersion as CcChainEncodingVersion, SignedAttestation as CcSignedAttestation,
    },
    supported_chains_primitives::SupportedChain as CcSupportedChain,
};

use attestor_primitives::{
    block::ContinuityProof, AttestationCheckpoint, AttestationData, AttestorId, AttestorStatus,
    BlsPublicKey, BlsSignature, ChainEncodingVersion, ChainKey, Digest, SignedAttestation,
};
use supported_chains_primitives::SupportedChain;
use vrf::{make_proof_of_inclusion, ProofOfInclusion};

pub use subxt::utils::{AccountId32, H256};

#[subxt::subxt(
    runtime_metadata_path = "artifacts/metadata.scale",
    substitute_type(
        path = "primitive_types::U256",
        with = "::subxt::utils::Static<crate::U256>"
    )
)]
pub mod cc3 {}

pub mod api;
pub mod events;
pub mod signer;

pub type Randomness = [u8; 32];

#[derive(Debug, Error)]
pub enum Error {
    #[error("Subxt error: {0}")]
    SubxtError(subxt::Error),
    #[error("Transaction timed out waiting for finalization")]
    TransactionTimeout,
    #[error("CC3 connection lost: {0:?}")]
    ConnectionError(Reconnect),

    #[error("Failed to get committee set size")]
    FailedToGetComitteSetSize,
    #[error("Failed to create proof of inclusion: {0}")]
    FailedToCreateProofOfInclusion(vrf::Error),

    #[error("Attestor not found in storage, register the attestor first and retry later")]
    NotRegistered,
    #[error("Caller cannot pay fees for the transaction")]
    CallerCannotPayFees,
    #[error("Caller doesn't have sufficient funds to execute the transaction: {0}")]
    CallerDoesntHaveSufficientFunds(subxt::error::TokenError),

    /// Reconnect was interrupted by a shutdown signal (`ctrl_c`). Callers should propagate this
    /// up so the task supervisor can exit cleanly instead of restarting the RPC dance.
    #[error("CC3 reconnect interrupted by shutdown signal")]
    ShuttingDown,
}

#[derive(Debug)]
/// Type-safe reconnection wrapper which prevents us from shooting ourselves in the foot by
/// reconnecting on a non-recoverable error.
pub struct Reconnect(subxt::Error);

// Filters out connection errors (ws drop, client restart...) and errors which cannot be recovered
// (insufficient funds, unregistered attestor).
impl From<subxt::Error> for Error {
    fn from(err: subxt::Error) -> Self {
        if is_transient_subxt(&err) {
            Self::ConnectionError(Reconnect(err))
        } else {
            Self::SubxtError(err)
        }
    }
}

/// Classifier shared by `From<subxt::Error>` so the logic has one source of truth.
///
/// Treats as recoverable (→ `Reconnect`):
///   * subxt's own `SubscriptionDropped` / `DisconnectedWillReconnect`
///   * jsonrpsee `Transport(_)` / `RestartNeeded(_)`
///   * jsonrpsee `Call(_)` whose message text indicates a server-side transient: the upstream
///     RPC sent us a structured JSON-RPC error back, but the message itself says e.g.
///     "connection going down", "service unavailable", "rate limit", etc. In those cases the
///     correct response is to back off + reconnect, not to crash the task.
fn is_transient_subxt(err: &subxt::Error) -> bool {
    match err {
        subxt::Error::Rpc(
            RpcError::SubscriptionDropped | RpcError::DisconnectedWillReconnect(_),
        ) => true,
        subxt::Error::Rpc(RpcError::ClientError(boxed)) => {
            let Some(jr) = boxed.downcast_ref::<JsonRpseeError>() else {
                return false;
            };
            match jr {
                JsonRpseeError::Transport(_) | JsonRpseeError::RestartNeeded(_) => true,
                JsonRpseeError::Call(obj) => is_transient_call_message(obj.message()),
                _ => false,
            }
        }
        _ => false,
    }
}

/// Server-side error messages that mean "the connection is temporarily unhealthy" rather than
/// "the call you made was invalid". Conservative — we only match phrases the chain or its load
/// balancer routinely emits at shutdown / overload. Anything not listed here surfaces to the
/// caller as `SubxtError(_)` and crashes the task instead of looping.
fn is_transient_call_message(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("connection")           // "Downstream connection unexpectedly closed", etc.
        || m.contains("going down")    // gateway graceful-shutdown banner
        || m.contains("shut")          // "shutting down", "shutdown"
        || m.contains("unavailable")   // 503 Service Unavailable, generic::unavailable
        || m.contains("rate limit")    // 429 from a fronting LB
        || m.contains("too many requests")
        || m.contains("timeout")       // upstream timeout (distinct from RPC::DEADLINE_EXCEEDED)
        || m.contains("deadline_exceeded")
}

impl From<vrf::Error> for Error {
    fn from(err: vrf::Error) -> Self {
        Self::FailedToCreateProofOfInclusion(err)
    }
}

impl From<subxt::error::TokenError> for Error {
    fn from(err: subxt::error::TokenError) -> Self {
        Self::CallerDoesntHaveSufficientFunds(err)
    }
}

/// Cc3 client that is configured with an url and keypair
///
/// Must connect to a node that has rpc and websocket enabled
/// - `url`: Creditcoin3 url (rpc + websocket enabled)
/// - `keypair`: Creditcoin3 keypair
///
/// The live subxt RPC connection lives behind an [`ArcSwap`] so that a single [`Client`] shared
/// across many tasks can be reconnected in place: once any holder calls [`Client::reconnect`],
/// every other holder picks up the fresh subxt `RpcClient` / `OnlineClient` on its next call.
///
/// `delay` is shared (`Arc<Mutex<_>>`) so backoff state survives across all `Client::clone()`
/// instances. Without this, V1's pattern of value-cloning a `Client` per `StreamCC3` (three
/// clones in the attestor) plus the validation worker's own field gives each holder an
/// independent exponential backoff — a chain-wide outage with N path failures means N dial
/// attempts every backoff window instead of one. The shared mutex keeps all reconnects on the
/// same timer regardless of how the `Client` was handed around.
#[derive(Clone)]
pub struct Client {
    signer: signer::CC3Signer,
    url: String,
    delay: Arc<Mutex<tokio_retry::strategy::ExponentialBackoff>>,

    inner: Arc<ArcSwap<ClientInner>>,
}

struct ClientInner {
    api: OnlineClient<SubstrateConfig>,
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
        let url = url.into();
        let delay = Arc::new(Mutex::new(util::exponential_retry_delay()));
        let inner = ClientInner::new(&url).await?;

        Ok(Self {
            signer,
            url,
            delay,
            inner: Arc::new(ArcSwap::new(Arc::new(inner))),
        })
    }

    /// Atomically replace the live subxt connection with a freshly-opened one.
    ///
    /// Bounded by [`util::MAX_RECONNECT_ATTEMPTS`] so a permanently-down RPC eventually
    /// surfaces an `Err` (the task supervisor crashes the process and k8s reschedules)
    /// rather than silently spinning forever.
    ///
    /// Cancellable via `tokio::signal::ctrl_c()` — on shutdown the in-progress reconnect
    /// returns `Err(Error::ShuttingDown)` instead of blocking the process exit until the
    /// RPC happens to come back. This restores the cancellation arm the V1 worker's helper
    /// had before this refactor.
    ///
    /// Takes `&self` so any task sharing a `Client` (or a clone of one — clones share the
    /// backoff mutex) can drive the reconnect without needing exclusive access.
    pub async fn reconnect(&self, Reconnect(err): Reconnect) -> Result<(), Error> {
        tracing::warn!(?err, "CC3 connection lost...");

        let mut attempt = 1;
        loop {
            // Snapshot the next delay from the shared backoff. The mutex is held only
            // long enough to advance the iterator — no awaits inside the critical section.
            let delay = {
                let mut guard = self.delay.lock().expect("reconnect delay mutex poisoned");
                guard.next().unwrap_or(util::MAX_DELAY)
            };
            let sleep = tokio::time::sleep(tokio_retry::strategy::jitter(delay));

            tokio::select! {
                () = sleep => {}
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("🔌 Reconnect interrupted by shutdown signal");
                    return Err(Error::ShuttingDown);
                }
            }

            match ClientInner::new(&self.url).await {
                Ok(inner) => {
                    self.inner.store(Arc::new(inner));
                    tracing::warn!(attempt, "Reconnected to CC3!");
                    return Ok(());
                }
                Err(Error::ConnectionError(err)) => {
                    tracing::warn!(attempt, ?err, "Failed to reconnect to CC3");
                }
                Err(err) => {
                    tracing::warn!(attempt, ?err, "Encountered fatal reconnection error");
                    return Err(err);
                }
            }

            attempt += 1;
        }
    }

    /// Reset the shared backoff timer to its initial state. Call this after any operation
    /// that proves the connection is healthy (a successful storage read or extrinsic
    /// submission) so the next transient failure starts from the minimum delay.
    pub fn reset_connection_delay(&self) {
        let mut guard = self.delay.lock().expect("reconnect delay mutex poisoned");
        *guard = util::exponential_retry_delay();
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
    /// The return value of this method references the inner atomic state of [`Client`] and should
    /// be short-lived: do not store this in a struct without creating an owned copy first in order
    /// to avoid locking!
    #[must_use]
    pub fn api(&self) -> impl arc_swap::access::Access<OnlineClient<SubstrateConfig>> + '_ {
        self.inner.map(|inner: &ClientInner| &inner.api)
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
            .inner
            .load()
            .api
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
            .inner
            .load()
            .api
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
            .inner
            .load()
            .api
            .storage()
            .at_latest()
            .await?
            .iter(address)
            .await?;

        while let Some(kv_res) = iter.next().await {
            supported_chains.push(kv_res?.value.into());
        }

        Ok(supported_chains)
    }

    /// Fetches the babe randomness from 2 epochs ago
    /// Returns the random a time + the current block number (where it was calculated from)
    pub async fn fetch_babe_randomness_two_epoch_ago(&self) -> Result<(Randomness, u64), Error> {
        let epoch_index = self
            .inner
            .load()
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
            tracing::info!("Epoch index is too low to fetch randomness");
            return Ok((Randomness::default(), two_epoch_ago));
        }
        tracing::info!("Fetching randomness for epoch index: {}", two_epoch_ago);

        let randomness = self
            .inner
            .load()
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
            .ok_or(Error::ConnectionError(Reconnect(subxt::Error::Rpc(
                // Babe randomness is no persisted by substrate in storage an might be missing for
                // the first two epochs after node crash.
                RpcError::RequestRejected(format!(
                    "Failed to get Babe VRF at epoch {two_epoch_ago}"
                )),
            ))))?;

        Ok((randomness, two_epoch_ago))
    }

    pub async fn get_current_epoch(&self) -> Result<u64, Error> {
        let epoch_index = self
            .inner
            .load()
            .api
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
            .inner
            .load()
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
            .inner
            .load()
            .api
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
            .inner
            .load()
            .api
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
            .inner
            .load()
            .api
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
            .inner
            .load()
            .api
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
            .inner
            .load()
            .api
            .tx()
            .create_signed(&tx, &self.signer.signing_keypair, params)
            .await?
            .submit_and_watch()
            .await
            .map_err(|e| {
                if util::is_fee_error(&e) {
                    Error::CallerCannotPayFees
                } else {
                    e.into()
                }
            })?;

        util::handle_tx(tx_progress, "Register Attestor").await
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
            .inner
            .load()
            .api
            .tx()
            .create_signed(&tx, &self.signer.signing_keypair, params)
            .await?
            .submit_and_watch()
            .await
            .map_err(|e| {
                if util::is_fee_error(&e) {
                    Error::CallerCannotPayFees
                } else {
                    e.into()
                }
            })?;

        util::handle_tx(tx_progress, "Chill Attestor").await
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
            .inner
            .load()
            .api
            .tx()
            .create_signed(&tx, &self.signer.signing_keypair, params)
            .await?
            .submit_and_watch()
            .await
            .map_err(|e| {
                if util::is_fee_error(&e) {
                    Error::CallerCannotPayFees
                } else {
                    e.into()
                }
            })?;

        util::handle_tx(tx_progress, "Unregister Attestor").await
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
            .inner
            .load()
            .api
            .tx()
            .sign_and_submit_then_watch_default(&tx, &self.signer.signing_keypair)
            .await
            .map_err(|e| {
                if util::is_fee_error(&e) {
                    Error::CallerCannotPayFees
                } else {
                    e.into()
                }
            })?;

        util::handle_tx(tx_progress, "Start Attesting").await
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

        tracing::info!(
            "Target set size: {target_sample_size}, committee set size: {committee_set_size}",
        );

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

        tracing::info!("committee set size: {committee_set_size}",);

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
            .inner
            .load()
            .api
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
            .inner
            .load()
            .api
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
            .inner
            .load()
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
    ) -> Result<Option<SignedAttestation<Digest, AccountId32>>, Error> {
        let storage_query = cc3::storage()
            .attestation()
            .attestations(chain_key, subxt::utils::H256::from(digest.0));

        let result = self
            .inner
            .load()
            .api
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
            .inner
            .load()
            .api
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
            .inner
            .load()
            .api
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
            .inner
            .load()
            .api
            .storage()
            .at_latest()
            .await?
            .iter(address)
            .await?;

        while let Some(kv_res) = iter.next().await {
            attestations.push(kv_res?.value.into());
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
            .inner
            .load()
            .api
            .storage()
            .at_latest()
            .await?
            .iter(address)
            .await?;

        while let Some(kv_res) = iter.next().await {
            let kv = kv_res?;
            if kv.key_bytes.len() < 8 {
                tracing::error!(
                    "Storage key for chainkey {} is less than 8 bytes, checkpoint: {:?}",
                    chain_key,
                    kv
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
                tracing::error!(
                    "Failed to get last 8 bytes of storage key for chainkey {}, checkpoint: {:?}",
                    chain_key,
                    kv
                );
            }
        }

        checkpoints.sort_by(|a: &AttestationCheckpoint, b: &AttestationCheckpoint| {
            // Highest to lowest by comparing b to a
            b.block_number.cmp(&a.block_number)
        });

        Ok(checkpoints)
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
            .inner
            .load()
            .api
            .tx()
            .create_signed(&tx, &self.signer.signing_keypair, params)
            .await?
            .submit_and_watch()
            .await
            .map_err(|e| {
                if util::is_fee_error(&e) {
                    Error::CallerCannotPayFees
                } else {
                    e.into()
                }
            })?;

        util::handle_tx(tx_progress, "Transfer").await
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
            .inner
            .load()
            .api
            .tx()
            .create_signed(&tx, &self.signer.signing_keypair, params)
            .await?
            .submit_and_watch()
            .await
            .map_err(|e| {
                if util::is_fee_error(&e) {
                    Error::CallerCannotPayFees
                } else {
                    e.into()
                }
            })?;

        util::handle_tx(tx_progress, "Set Balance").await
    }

    pub async fn get_free_balance(&self, account: &AccountId32) -> Result<u128, Error> {
        let storage_query = cc3::storage().system().account(account);
        let account_info = self
            .inner
            .load()
            .api
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;
        Ok(account_info.map_or(0, |info| info.data.free))
    }

    pub async fn get_account_nonce(&self) -> Result<u64, Error> {
        let nonce = self
            .inner
            .load()
            .api
            .tx()
            .account_nonce(&self.signer.account_id())
            .await?;

        Ok(nonce)
    }

    pub async fn get_attestor_active_set(&self, chain_key: u64) -> Result<Vec<AccountId32>, Error> {
        let storage_query = cc3::storage().attestation().active_attestors(chain_key);
        let result = self
            .inner
            .load()
            .api
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

        let tx_progress = self
            .inner
            .load()
            .api
            .tx()
            .create_signed(&tx, &self.signer.signing_keypair, params)
            .await?
            .submit_and_watch()
            .await
            .map_err(|e| {
                if util::is_fee_error(&e) {
                    Error::CallerCannotPayFees
                } else {
                    e.into()
                }
            })?;

        util::handle_tx(tx_progress, "Set Attestation Chain Genesis Block Number").await
    }

    pub async fn get_attestation_chain_genesis_block_number(
        &self,
        chain_key: ChainKey,
    ) -> Result<u64, Error> {
        let storage_query = cc3::storage()
            .attestation()
            .attestation_chain_genesis_block_number(chain_key);

        let result = self
            .inner
            .load()
            .api
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        Ok(result.unwrap_or_default())
    }
}

impl ClientInner {
    async fn new(url: &str) -> Result<Self, Error> {
        let rpc = RpcClient::from_insecure_url(url).await?;
        let api = OnlineClient::<SubstrateConfig>::from_rpc_client(rpc).await?;

        Ok(Self { api })
    }
}

mod util {
    /// Timeout for waiting on extrinsic finalization.
    /// Set to 120 seconds which is around 8 blocks on a 15 second block time.
    const FINALIZATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

    /// This is a fallback error message that we can use to detect insufficient funds errors in the absence of a more structured error from the RPC layer.
    /// Sourced from: <https://github.com/paritytech/polkadot-sdk/blob/06bded7ab7ac6a50e0aeba48c0f7f5ca548c3573/substrate/primitives/runtime/src/transaction_validity.rs#L116>
    const INABILITY_TO_PAY_SOME_FEE_MSG: &str = "Inability to pay some fee";

    pub const MAX_DELAY: std::time::Duration = std::time::Duration::from_millis(5_000);

    pub fn exponential_retry_delay() -> tokio_retry::strategy::ExponentialBackoff {
        tokio_retry::strategy::ExponentialBackoff::from_millis(100).max_delay(MAX_DELAY)
    }

    pub async fn handle_tx(
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

    pub fn is_fee_error(e: &subxt::Error) -> bool {
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
