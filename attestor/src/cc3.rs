use anyhow::Result;
use bls_signatures::{PrivateKey, Serialize as BlsSerialize};
use creditcoin3_attestor_gossip::{Attestation, Topic};
use jsonrpsee_core::{client::ClientT, params::ArrayParams, rpc_params};
use kameo::{
    actor::Actor,
    message::{Context, Message},
};
use serde::{Deserialize, Serialize};
use sp_core::H256;
use thiserror::Error;
use tracing::{debug, error, info, warn};

use cc_client::Client as CcClient;

use attestor_primitives::{AttestationData, BlsPublicKey, ChainId};

#[subxt::subxt(runtime_metadata_path = "artifacts/metadata.scale")]
pub mod cc3 {}

pub type Randomness = [u8; 32];

#[derive(Debug, Clone)]
/// Cc3 client that is configured with an url and keypair
/// Must connect to a node that has rpc and websocket enabled
/// - `cc_client`: Creditcoin3 client
/// - `bls_keypair`: BLS keypair
pub struct Client {
    pub cc_client: CcClient,
    pub bls_keypair: PrivateKey,
    pub attestation_interval: u64,
}

impl Client {
    pub fn get_bls_pubkey(&self) -> Result<BlsPublicKey, Error> {
        let pubkey_bytes = self.bls_keypair.public_key().as_bytes();

        let mut pubkey = [0; 48];

        if pubkey_bytes.len() != 48 {
            return Err(Error::InvalidBlsKey);
        }

        pubkey.copy_from_slice(&pubkey_bytes[0..48]);

        Ok(pubkey)
    }
}

impl<'a> Client {
    /// Create a new instance of cc3 client
    /// - `url`: rpc url of a creditcoin node
    /// - `key`: secret phrase for a creditcoin key
    pub async fn new(
        url: impl Into<String> + Clone,
        key: &'a str,
        chain_id: ChainId,
        // private_key: &[u8; 32],
    ) -> Result<Self> {
        let cc_client = CcClient::new(url, key)?;

        // Derive bls key from secret seed
        let bls_keypair = PrivateKey::new(key.as_bytes());

        let attestation_interval = cc_client
            .chain_attestation_interval(chain_id)
            .await?
            .ok_or(Error::FailedToGetAttestationInterval)?;

        Ok(Self {
            cc_client,
            bls_keypair,
            attestation_interval,
        })
    }

    /// Init the client, this bootstraps registration if not registered already
    pub async fn init(&self) -> Result<()> {
        let is_attestor_member = self.cc_client.check_attestors_membership().await?;

        if !is_attestor_member {
            debug!("Registration in progress... Please wait...");
            self.register().await?;
        }

        info!("Attestator ready to start!");

        Ok(())
    }

    /// Register to the attestation pallet
    pub async fn register(&self) -> Result<()> {
        self.cc_client
            .register_attestor(self.get_bls_pubkey()?)
            .await
    }

    pub async fn chain_attestation_interval(&self, chain_id: ChainId) -> Result<Option<u64>> {
        let api = self.get_substrate_client().await?;

        let storage_query = cc3::storage()
            .attestation()
            .chain_attestation_interval(chain_id);

        let result = api
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        Ok(result)
    }
}

impl Actor for Client {}

// AttestationSubmit is a message that can be sent to submit an attestation over rpc to the cc3 node
// It holds the attestation data to be signed by the attestor before submitting
pub struct AttestationSubmit<H> {
    pub attestation: AttestationData<H>,
}

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

impl<H> Message<AttestationSubmit<H>> for Client
where
    H: Serialize + AsRef<[u8]> + Send + Sync + std::fmt::Debug,
{
    type Reply = Result<(), Error>;

    /// Main attestation handler
    /// This function will check eligibility for submitting attestations if eligible it will sign and submit to cc3
    async fn handle(
        &mut self,
        msg: AttestationSubmit<H>,
        _ctx: Context<'_, Self, Self::Reply>,
    ) -> Self::Reply {
        let vrf_output = self.cc_client.sign_babe_vrf().await.map_err(|e| {
            error!("Error signing babe vrf: {:?}", e);
            Error::FailedToSignBabeVrf
        })?;

        let is_attestor_member =
            self.cc_client
                .check_attestors_membership()
                .await
                .map_err(|e| {
                    error!("Error checking membership: {:?}", e);
                    Error::FailedToCheckEligibility
                })?;

        if !is_attestor_member {
            error!("Attestor is not valid at current timeframe, please exit.");
            return Err(Error::InvalidAttestor);
        };

        // Sign the attestation data
        let signature = self.cc_client.sign(&msg.attestation.serialize());

        // sign attestation data with bls key
        let signature_bls = self.bls_private_key.sign(msg.attestation.serialize());

        info!("attestation to submit: {:?}", msg.attestation);
        // Create final attestation object
        let attestation = Attestation {
            attestation_data: msg.attestation,
            attestor: self.cc_client.get_attestor_id(),
            topic: Topic::new(1),
            vrf_output,
            signature: sp_core::sr25519::Signature::from_raw(signature.0),
            signature_bls: attestor_primitives::bls::WrapEncode(signature_bls),
        };

        // Submit the attestation to the chain
        let rpc_client = self.cc_client.get_rpc_client().map_err(|e| {
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

pub struct GetLastDigest {
    pub chain_id: ChainId,
}

impl Message<GetLastDigest> for Client {
    type Reply = Result<H256, Error>;

    async fn handle(
        &mut self,
        msg: GetLastDigest,
        _ctx: Context<'_, Self, Self::Reply>,
    ) -> Self::Reply {
        let last_digest = self
            .cc_client
            .fetch_last_digest(msg.chain_id)
            .await
            .map_err(|e| {
                error!("Error checking latest digest: {:?}", e);
                Error::FailedToFetchDigest
            })?;

        let last_digest = last_digest.unwrap_or(H256::zero());
        info!("Last digest: {:?}", last_digest);

        Ok(last_digest)
    }
}
