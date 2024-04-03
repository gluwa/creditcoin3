use std::str::FromStr;

use anyhow::Result;
use jsonrpsee_core::{client::ClientT, params::ArrayParams, rpc_params};
use jsonrpsee_http_client::{HttpClient, HttpClientBuilder};
use kameo::{Actor, Message};
use serde::{Deserialize, Serialize};
use subxt::{OnlineClient, SubstrateConfig};
use subxt_signer::{sr25519::Keypair, SecretUri};
use tracing::{debug, error, info};

use creditcoin3_attestor_gossip::{Attestation, AttestorId, Topic};

use crate::attestation::AttestationData;

#[subxt::subxt(runtime_metadata_path = "artifacts/metadata.scale")]
pub mod cc3 {}

pub type Randomness = [u8; 32];

#[derive(Debug, Clone)]
pub struct Client {
    pub rpc_client: HttpClient,
    pub keypair: Keypair,
}

impl<'a> Client {
    /// Create a new instance of cc3 client
    /// - url: rpc url of a creditcoin node
    /// - key: secret phrase for a creditcoin key
    pub fn new(url: &'a str, key: &'a str) -> Result<Self> {
        let secret_uri = SecretUri::from_str(key)?;
        let keypair = Keypair::from_uri(&secret_uri)?;

        let rpc_client = HttpClientBuilder::new().build(url)?;

        Ok(Self {
            keypair,
            rpc_client,
        })
    }

    pub async fn get_substrate_client(&self) -> Result<OnlineClient<SubstrateConfig>> {
        Ok(OnlineClient::<SubstrateConfig>::from_url("ws://localhost:9944").await?)
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
            .unwrap();

        Ok(result)
    }

    pub async fn can_attest(&self) -> Result<bool> {
        let _api = self.get_substrate_client().await?;

        // Query pallet storage and check

        Ok(true)
    }

    /// sign_babe_vrf signs babe's author vrf randomness with the configured key and returns the output as integer
    /// the method extracts the S component bytes from the signature. The bytes of the S component are converted into a u64 integer using little-endian byte order.
    pub async fn sign_babe_vrf(&self) -> Result<u64> {
        let randomness = self.fetch_babe_randomness().await?.unwrap_or_default();
        info!("Babe VRF Randomness: {:?}", randomness);

        // Sign the randomness
        let signature = self.keypair.sign(&randomness);

        // Extract the `S` component bytes of the signature
        let s_component_bytes = &signature.0[32..64];

        // Convert `S` component bytes to an integer
        let s_component_integer = u64::from_le_bytes([
            s_component_bytes[0],
            s_component_bytes[1],
            s_component_bytes[2],
            s_component_bytes[3],
            s_component_bytes[4],
            s_component_bytes[5],
            s_component_bytes[6],
            s_component_bytes[7],
        ]);
        info!("S Component Bytes: {:?}", s_component_bytes);
        info!("S Component Integer: {:?}", s_component_integer);

        Ok(s_component_integer)
    }

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Error {
    CannotAttest,
    FailedToSubmit,
    FailedToSignBabeVrf,
}

impl Message<Client> for AttestationSubmit {
    type Reply = Result<(), Error>;

    async fn handle(self, state: &mut Client) -> Self::Reply {
        let vrf_output = state.sign_babe_vrf().await.map_err(|e| {
            error!("Error signing babe vrf: {:?}", e);
            Error::FailedToSignBabeVrf
        })?;

        // TODO: Check if the signature value is above/below some threshold defined on chain

        // Sign the attestation data
        let signature = state.keypair.sign(&self.attestation.serialize());

        // Create final attestation object
        let attestation = Attestation {
            attestor: state.get_attestor_id(),
            round: 1,
            header_hash: self.attestation.header_hash,
            header_number: self.attestation.header_number,
            topic: Topic::new(1),
            vrf_output,
            signature: sp_core::sr25519::Signature::from_raw(signature.0),
        };

        // Submit the attestation to the chain
        let _ = state
            .rpc_client
            .request::<(), ArrayParams>("attestor_submitAttestation", rpc_params!(attestation))
            .await
            .map_err(|e| {
                error!("error submitting rpc: {:?}", e);
                Error::FailedToSubmit
            });

        debug!("Attestation submitted");

        Ok(())
    }
}
