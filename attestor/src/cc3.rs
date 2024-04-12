use std::str::FromStr;

use alloy::primitives::U256;
use anyhow::Result;
use jsonrpsee_core::{client::ClientT, params::ArrayParams, rpc_params};
use jsonrpsee_http_client::{HttpClient, HttpClientBuilder};
use kameo::{Actor, Message};
use serde::{Deserialize, Serialize};
use subxt::{OnlineClient, SubstrateConfig};
use subxt_signer::{sr25519::Keypair, SecretUri};
use thiserror::Error;
use tracing::{debug, error, info};

use creditcoin3_attestor_gossip::{Attestation, AttestorId, Topic};

use attestor_primitives::AttestationData;

#[subxt::subxt(runtime_metadata_path = "artifacts/metadata.scale")]
pub mod cc3 {}

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
    pub fn new(url: impl Into<String> + Clone, key: &'a str) -> Result<Self> {
        let secret_uri = SecretUri::from_str(key)?;
        let keypair = Keypair::from_uri(&secret_uri)?;

        Ok(Self {
            url: url.into(),
            keypair,
        })
    }

    /// Init the client, this bootstraps registration if not registered already
    pub async fn init(&self) -> Result<()> {
        let is_attestor_member = self.check_attestors_membership().await?;

        if !is_attestor_member {
            debug!("Registration in progress... Please wait...");
            self.register().await?;
        }

        info!("Attestator ready to start!");

        Ok(())
    }

    /// Get's a substrate client over websocket to the configured url
    pub async fn get_substrate_client(&self) -> Result<OnlineClient<SubstrateConfig>> {
        debug!("connecting to {}", replace_http_with_ws(&self.url));
        Ok(OnlineClient::<SubstrateConfig>::from_url(replace_http_with_ws(&self.url)).await?)
    }

    /// Get's an rpc http client to the configured url
    fn get_rpc_client(&self) -> Result<HttpClient> {
        Ok(HttpClientBuilder::new().build(&self.url)?)
    }

    /// Fetches the babe author VRF (Verifiable Random Functions) randomness at current block
    pub(crate) async fn fetch_babe_randomness(&self) -> Result<Option<Randomness>> {
        let api = self.get_substrate_client().await?;

        let storage_query = cc3::storage().babe().author_vrf_randomness();

        // Probably want to get it from 2 epochs ago (need to fetch current epoch and epoch duration for that)
        let result = api
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?
            .and_then(|v| v);

        Ok(result)
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
    pub async fn register(&self) -> Result<()> {
        let api = self.get_substrate_client().await?;

        let tx = cc3::tx().attestation().register_attestor();

        let ext = api
            .tx()
            .sign_and_submit_then_watch_default(&tx, &self.keypair)
            .await?
            .wait_for_finalized_success()
            .await?;

        let hash = ext.block_hash();
        debug!("Registration extrinsic submitted with hash: {:?}", hash);

        Ok(())
    }

    /// `sign_babe_vrf` signs babe's author vrf randomness with the configured key and returns the output as integer
    /// the method extracts the S component bytes from the signature. The bytes of the S component are converted into a u64 integer using little-endian byte order.
    pub async fn sign_babe_vrf(&self) -> Result<U256, Error> {
        let randomness = self
            .fetch_babe_randomness()
            .await
            .map_err(|e| {
                error!("Error getting babe vrf output: {:?}", e);
                Error::FailedToGetBabeVrf
            })?
            .ok_or(Error::BabeVrfOuputInvalid)?;
        info!("Babe VRF Randomness: {}", hex::encode(randomness));

        let randomness_as_u256 = U256::from_le_bytes(randomness);

        // Sign the randomness
        let signature = self.keypair.sign(&randomness);

        // Convert `S` component bytes to a [u8; 32] array
        let mut s_component_array = [0; 32];
        s_component_array.copy_from_slice(&signature.0[32..64]);

        // Convert `S` component bytes to an integer
        let signature_output_as_u256 = U256::from_le_bytes(s_component_array);

        info!(
            "Signature output is above or below threshold: {}",
            signature_output_as_u256 > randomness_as_u256
        );

        Ok(signature_output_as_u256)
    }

    #[must_use]
    pub fn get_attestor_id(&self) -> AttestorId {
        AttestorId::from_public(self.keypair.public_key().0)
    }
}

impl Actor for Client {}

// AttestationSubmit is a message that can be sent to submit an attestation over rpc to the cc3 node
// It holds the attestation data to be signed by the attestor before submitting
pub struct AttestationSubmit {
    pub attestation: AttestationData,
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
    #[error("Invalid attestor")]
    InvalidAttestor,
    #[error("Failed to get cc3 RPC client")]
    FailedToGetRPcClient,
    #[error("Failed to get comittee set size")]
    FailedToGetComitteSetSize,
}

impl Message<AttestationSubmit> for Client {
    type Reply = Result<(), Error>;

    /// Main attestation handler
    /// This function will check eligibility for submitting attestations if eligible it will sign and submit to cc3
    async fn handle(&mut self, msg: AttestationSubmit) -> Self::Reply {
        let vrf_output = self.sign_babe_vrf().await.map_err(|e| {
            error!("Error signing babe vrf: {:?}", e);
            Error::FailedToSignBabeVrf
        })?;

        let is_attestor_member = self.check_attestors_membership().await.map_err(|e| {
            error!("Error checking membership: {:?}", e);
            Error::FailedToCheckEligibility
        })?;

        if !is_attestor_member {
            error!("Attestor is not valid at current timeframe, please exit.");
            return Err(Error::InvalidAttestor);
        };

        // Sign the attestation data
        let signature = self.keypair.sign(&msg.attestation.serialize());

        // Create final attestation object
        let attestation = Attestation {
            attestor: self.get_attestor_id(),
            round: 1,
            header_hash: hex::encode(msg.attestation.header_hash),
            header_number: msg.attestation.header_number,
            tx_root: msg.attestation.tx_root,
            rx_root: msg.attestation.rx_root,
            topic: Topic::new(1),
            vrf_output: sp_core::U256::from_little_endian(vrf_output.as_le_slice()),
            signature: sp_core::sr25519::Signature::from_raw(signature.0),
        };

        // Submit the attestation to the chain
        let rpc_client = self.get_rpc_client().map_err(|e| {
            error!("Error getting rpc client: {:?}", e);
            Error::FailedToGetRPcClient
        })?;

        let _ = rpc_client
            .request::<(), ArrayParams>("attestor_submitAttestation", rpc_params!(attestation))
            .await
            .map_err(|e| {
                error!("error submitting rpc: {:?}", e);
                Error::FailedToSubmit
            });

        info!("Attestation submitted");

        Ok(())
    }
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
