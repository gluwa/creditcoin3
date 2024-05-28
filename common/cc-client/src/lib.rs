use std::str::FromStr;

use anyhow::Result;
use creditcoin3_attestor_gossip::{AttestorId, VrfOutput};
use jsonrpsee_http_client::{HttpClient, HttpClientBuilder};
use serde::{Deserialize, Serialize};
use sp_core::{H256, U256};
use subxt::utils::AccountId32;
use subxt::{OnlineClient, SubstrateConfig};
use subxt_signer::{
    sr25519::{Keypair, Signature},
    SecretUri,
};
use thiserror::Error;
use tracing::{debug, error, info};

use attestor_primitives::{BlsPublicKey, ChainId, Digest};

#[subxt::subxt(runtime_metadata_path = "artifacts/metadata.scale")]
pub mod cc3 {}

use cc3::runtime_types::pallet_prover::types::Prover;

pub type Randomness = [u8; 32];

#[derive(Debug, Clone)]
/// Cc3 client that is configured with an url and keypair
/// Must connect to a node that has rpc and websocket enabled
/// - `url`: Creditcoin3 url (rpc + websocket enabled)
/// - `keypair`: Creditcoin3 keypair
pub struct Client {
    pub url: String,
    pub keypair: Keypair,
}

impl<'a> Client {
    /// Create a new instance of cc3 client
    /// - `url`: rpc url of a creditcoin node
    /// - `key`: secret phrase for a creditcoin key
    pub fn new(
        url: impl Into<String> + Clone,
        key: &'a str,
        // private_key: &[u8; 32],
    ) -> Result<Self> {
        let secret_uri = SecretUri::from_str(key)?;
        let keypair = Keypair::from_uri(&secret_uri)?;

        Ok(Self {
            url: url.into(),
            keypair,
        })
    }

    pub fn sign(&self, message: &[u8]) -> Signature {
        self.keypair.sign(message)
    }

    // /// Init the client, this bootstraps registration if not registered already
    // pub async fn init(&self) -> Result<()> {
    //     let is_attestor_member = self.check_attestors_membership().await?;

    //     if !is_attestor_member {
    //         debug!("Registration in progress... Please wait...");
    //         self.register().await?;
    //     }

    //     info!("Attestator ready to start!");

    //     Ok(())
    // }

    /// Get's a substrate client over websocket to the configured url
    pub async fn get_substrate_client(&self) -> Result<OnlineClient<SubstrateConfig>> {
        debug!("connecting to {}", replace_http_with_ws(&self.url));
        Ok(OnlineClient::<SubstrateConfig>::from_url(replace_http_with_ws(&self.url)).await?)
    }

    /// Get's an rpc http client to the configured url
    pub fn get_rpc_client(&self) -> Result<HttpClient> {
        Ok(HttpClientBuilder::new().build(&self.url)?)
    }

    /// Fetches the babe randomness from 2 epochs ago
    /// Returns the random at that time + the current block number (where it was calculated from)
    pub(crate) async fn fetch_babe_randomness(&self) -> Result<(Option<Randomness>, H256)> {
        let api = self.get_substrate_client().await?;

        // Get epoch duration
        let epoch_duration = api
            .constants()
            .at(&cc3::constants().babe().epoch_duration())?;

        // Get current block number
        let current_block_number = api
            .storage()
            .at_latest()
            .await?
            .fetch(&cc3::storage().system().number())
            .await?
            .unwrap_or_default();

        // Calculate a block number that falls into the range of 2 epoch ago
        // current block - (epoch duration in block * 2)
        let block_to_query = current_block_number
            .checked_sub(u32::try_from(epoch_duration * 2)?)
            .unwrap_or(1);

        let block_hash_to_query = api
            .storage()
            .at_latest()
            .await?
            .fetch(&cc3::storage().system().block_hash(block_to_query))
            .await?
            .ok_or(Error::FailedToGetBabeVrf)?;

        info!("Getting babe randomness at block: {block_to_query}");
        // Probably want to get it from 2 epochs ago (need to fetch current epoch and epoch duration for that)
        let randomness = api
            .storage()
            .at(block_hash_to_query)
            .fetch(&cc3::storage().babe().randomness())
            .await?;

        Ok((randomness, block_hash_to_query))
    }

    pub async fn _fetch_comittee_size(&self) -> Result<u32> {
        let api = self.get_substrate_client().await?;

        let storage_query = cc3::storage().attestation().comittee_set_size();

        let result = api
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?
            .ok_or(Error::FailedToGetComitteSetSize)?;

        Ok(result)
    }

    pub async fn fetch_last_digest(&self, chain_id: ChainId) -> Result<Option<Digest>> {
        let api = self.get_substrate_client().await?;

        let storage_query = cc3::storage().attestation().last_digest(chain_id);

        let result = api
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        Ok(result)
    }

    /// Check the clients membership in the attestor pallet
    pub async fn check_attestors_membership(&self) -> Result<bool> {
        let api = self.get_substrate_client().await?;

        let storage_query = cc3::storage()
            .attestation()
            .attestors(subxt::utils::AccountId32::from(self.keypair.public_key()));

        let result = api
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?
            .is_some();

        Ok(result)
    }

    /// Register to the attestation pallet
    pub async fn register_attestor(&self, bls_public_key: BlsPublicKey) -> Result<()> {
        let api = self.get_substrate_client().await?;

        // let public_key = self.get_bls_pubkey()?;
        let tx = cc3::tx().attestation().register_attestor(bls_public_key);

        let ext = api
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
    pub async fn check_provers_membership(&self) -> Result<bool> {
        let api = self.get_substrate_client().await?;

        let storage_query = cc3::storage()
            .prover()
            .provers(AccountId32(self.keypair.public_key().0));

        let result = api
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
        let api = self.get_substrate_client().await?;

        let tx = cc3::tx().prover().register_prover(Prover {
            nickname: nickname.into(),
        });

        let ext = api
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
        let (randomness, block_hash) = self.fetch_babe_randomness().await.map_err(|e| {
            error!("Error getting babe vrf output: {:?}", e);
            Error::FailedToGetBabeVrf
        })?;

        let randomness = if let Some(r) = randomness {
            r
        } else {
            info!(
                "Randomness is not initialised at {:?}, making default hash",
                block_hash
            );
            H256::zero().0
        };

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
            block_hash,
        })
    }

    #[must_use]
    pub fn get_attestor_id(&self) -> AttestorId {
        AttestorId::from_public(self.keypair.public_key().0)
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
    #[error("Failed to get cc3 RPC client")]
    FailedToGetRPcClient,
    #[error("Failed to get comittee set size")]
    FailedToGetComitteSetSize,
}

/// Helper function to format a http(s) endpoint to a ws(s) endpoint
fn replace_http_with_ws(url: &str) -> String {
    // Check if the URL starts with "http://" or "https://"
    if let Some(stripped) = url.strip_prefix("http://") {
        format!("ws://{stripped}") // Replace "http://" with "ws://"
    } else if let Some(stripped) = url.strip_prefix("https://") {
        format!("wss://{stripped}") // Replace "https://" with "wss://"
    } else {
        url.to_string()
    }
}
