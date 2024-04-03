use std::str::FromStr;

use anyhow::Result;
use jsonrpsee_core::{client::ClientT, params::ArrayParams, rpc_params};
use jsonrpsee_http_client::{HttpClient, HttpClientBuilder};
use kameo::{Actor, Message};
use serde::{Deserialize, Serialize};
use subxt::{OnlineClient, SubstrateConfig};
use subxt_signer::{sr25519::Keypair, SecretUri};
use tracing::{debug, error};

use creditcoin3_attestor_gossip::{Attestation, AttestorId};

#[subxt::subxt(runtime_metadata_path = "artifacts/metadata.scale")]
pub mod cc3 {}

pub type Randomness = [u8; 32];

#[derive(Debug, Clone)]
pub struct Client {
    pub rpc_client: HttpClient,
    pub keypair: Keypair,
}

impl<'a> Client {
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
        Ok(OnlineClient::<SubstrateConfig>::from_url("http://localhost:9944").await?)
    }

    pub(crate) async fn _fetch_babe_randomness(&self) -> Result<Option<Randomness>> {
        let api = self.get_substrate_client().await?;

        let storage_query = cc3::storage().babe().randomness();

        // Probably want to get it from 2 epochs ago (need to fetch current epoch and epoch duration for that)
        let result = api
            .storage()
            .at_latest()
            .await?
            .fetch(&storage_query)
            .await?;

        Ok(result)
    }

    pub async fn can_attest(&self) -> Result<bool> {
        let _api = self.get_substrate_client().await?;

        // Query pallet storage and check

        Ok(true)
    }
}

impl Actor for Client {}

// AttestationSubmit is a message that can be sent to submit an attestation over rpc to the cc3 node
pub struct AttestationSubmit<H: Serialize> {
    pub attestation: Attestation<H>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Error {
    CannotAttest,
    FailedToSubmit,
}

impl<H> Message<Client> for AttestationSubmit<H>
where
    H: Send + Sync + 'static + Serialize,
{
    type Reply = Result<(), Error>;

    async fn handle(self, state: &mut Client) -> Self::Reply {
        // if !state.can_attest().await.map_err(|_| Error::CannotAttest)? {
        //     info!("Cannot attest yet");
        //     return Err(Error::CannotAttest);
        // }

        let _ = state
            .rpc_client
            .request::<(), ArrayParams>("attestor_submitAttestation", rpc_params!(self.attestation))
            .await
            .map_err(|e| {
                error!("error submitting rpc: {:?}", e);
                Error::FailedToSubmit
            });

        debug!("Attestation submitted");

        Ok(())
    }
}

/// GetAttestorId is a message that can be sent to cc3 in order to get the attestor id
pub struct GetAttestorId {}

impl Message<Client> for GetAttestorId {
    type Reply = Result<AttestorId>;

    async fn handle(self, state: &mut Client) -> Self::Reply {
        let id = AttestorId::from_public(state.keypair.public_key().0);
        Ok(id)
    }
}
