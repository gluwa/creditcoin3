use anyhow::Result;
use bls_signatures::{PrivateKey, Serialize as BlsSerialize};
use creditcoin3_attestor_gossip::{Attestation, AttestorId, Topic};
use exponential_backoff::Backoff;
use kameo::{
    actor::Actor,
    message::{Context, Message},
};
use serde::{Deserialize, Serialize};
use sp_core::H256;
use std::{thread, time::Duration};
use tracing::{debug, error, info, warn};

use cc_client::Client as CcClient;

use attestor_primitives::{
    Attestation as AttestationPrimitive, BlsPublicKey, BlsSignature, ChainId,
};

#[subxt::subxt(runtime_metadata_path = "artifacts/metadata.scale")]
pub mod cc3 {}

pub type Randomness = [u8; 32];

#[derive(Debug, Clone)]
/// Cc3 client that is configured with an url and keypair
/// Must connect to a node that has rpc and websocket enabled
/// - `cc_client`: Creditcoin3 client
/// - `bls_keypair`: BLS keypair
/// - `attestation_interval`: Attestation interval for the chain
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

    pub fn proof_of_possession(&self) -> Result<BlsSignature, Error> {
        let pubkey = self.get_bls_pubkey()?;

        self.bls_keypair.sign(pubkey).as_bytes()[..96]
            .try_into()
            .map_err(|_| {
                error!("Error converting proof of possession to bytes");
                Error::InvalidProofOfPossession
            })
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
            .register_attestor(self.get_bls_pubkey()?, self.proof_of_possession()?)
            .await
    }

    pub async fn sign_attestation<H>(
        &self,
        attestation: AttestationPrimitive<H>,
    ) -> Result<Attestation<H, AttestorId>>
    where
        H: Serialize + AsRef<[u8]> + Send + Sync + std::fmt::Debug + Clone,
    {
        let msg = attestation.serialize();
        // Sign the attestation data
        let signature = self.cc_client.sign(&msg);

        // sign attestation data with bls key
        let signature_bls = self.bls_keypair.sign(msg);

        // Sign the VRF output
        let vrf_output = self.cc_client.sign_babe_vrf().await.map_err(|e| {
            error!("Error signing babe vrf: {:?}", e);
            Error::FailedToSignBabeVrf
        })?;

        info!("attestation to submit: {:?}", attestation);

        // Create final attestation object
        Ok(Attestation {
            attestation_data: attestation,
            attestor: self.cc_client.get_attestor_id(),
            topic: Topic::new(1),
            vrf_output,
            signature: sp_core::sr25519::Signature::from_raw(signature.0),
            signature_bls: attestor_primitives::bls::WrapEncode(signature_bls),
        })
    }
}

impl Actor for Client {}

// AttestationSubmit is a message that can be sent to submit an attestation over rpc to the cc3 node
// It holds the attestation data to be signed by the attestor before submitting
pub struct AttestationSubmit<H> {
    pub attestation: Option<AttestationPrimitive<H>>,
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
    H: Serialize + AsRef<[u8]> + Send + Sync + std::fmt::Debug + Clone,
{
    type Reply = Result<(), Error>;

    /// Main attestation handler
    /// This function will check eligibility for submitting attestations if eligible it will sign and submit to cc3
    async fn handle(
        &mut self,
        msg: AttestationSubmit<H>,
        _ctx: Context<'_, Self, Self::Reply>,
    ) -> Self::Reply {
        // If attestation is none, return
        if msg.attestation.is_none() {
            warn!("Attestation data is none, skipping...");
            return Ok(());
        }

        let attestation = self
            .sign_attestation(msg.attestation.unwrap())
            .await
            .map_err(|e| {
                error!("Error signing attestation: {:?}", e);
                Error::FailedToSignBabeVrf
            })?;

        submit_attestation(self.cc_client.clone(), attestation)
            .await
            .map_err(|e| {
                error!("Error submitting attestation: {:?}", e);
                Error::FailedToSubmit
            })?;

        Ok(())
    }
}

pub async fn submit_attestation<H, AccountId>(
    cc_client: CcClient,
    mut attestation: Attestation<H, AccountId>,
) -> Result<()>
where
    H: Serialize + AsRef<[u8]> + Send + Sync + std::fmt::Debug + Clone,
    AccountId: Serialize + Send + Sync + std::fmt::Debug + Clone,
{
    // check we can submit this attestation according to the interval
    let attestation_interval = cc_client
        .chain_attestation_interval(attestation.attestation_data.chain_id)
        .await
        .map_err(|_e| {
            error!(
                "Error getting attestation interval for chain: {}",
                attestation.attestation_data.chain_id
            );
            Error::FailedToGetAttestationInterval
        })?
        .ok_or(Error::FailedToGetAttestationInterval)?;

    if attestation.attestation_data.header_number % attestation_interval != 0 {
        warn!("Skipping Attestation because it's not in the configured interval for this chain");
        return Ok(());
    };

    let is_attestor_member = cc_client.check_attestors_membership().await?;

    if !is_attestor_member {
        warn!("Attestor is not valid at current timeframe, skipping...");
        return Ok(());
    };

    // Get the digest of the attestation
    let attestation_digest = attestation.attestation_data.digest();

    // check if attestation already exists
    // if yes, don't submit
    let exists = cc_client
        .chain_attestation_exists(attestation.attestation_data.chain_id, attestation_digest)
        .await?;

    if exists {
        warn!("Attestation already exists, skipping...");
        return Ok(());
    }

    // Get the last digest from the chain
    // and set it as the previous digest of the attestation
    let prev_digest = cc_client
        .fetch_last_digest(attestation.attestation_data.chain_id)
        .await?;
    attestation.attestation_data.prev_digest = prev_digest;

    let mut inclusion = false;
    while !inclusion {
        info!("Trying to submit attestation...");
        // Submit the attestation to the chain
        cc_client
            .submit_attestation(attestation.clone())
            .await
            .map_err(|e| {
                error!("Error submitting attestation: {:?}", e);
                Error::FailedToSubmit
            })?;

        inclusion = check_attestation_inclusion(
            cc_client.clone(),
            attestation.attestation_data.chain_id,
            attestation_digest,
        )
        .await?;
    }

    info!("✅ Attestation with digest {attestation_digest} included in chain");

    Ok(())
}

/// Check if the attestation is included in the chain
/// - `cc_client`: Creditcoin3 client
/// - `chain_id`: Chain id
/// - `attestation_digest`: Attestation digest
/// Returns a boolean indicating if the attestation is included in the chain
/// It retries 10 times with 6 seconds interval
pub async fn check_attestation_inclusion(
    cc_client: CcClient,
    chain_id: ChainId,
    attestation_digest: H256,
) -> Result<bool> {
    let retries = 10;
    let min = Duration::from_secs(6);
    // Retry 10 times with 6 seconds interval (blocktime is 6 seconds)
    let backoff = Backoff::new(retries, min, min);

    info!("Validating attestation submission now...");
    for duration in &backoff {
        // get last digest from cc3
        let last_digest = cc_client.fetch_last_digest(chain_id).await?;

        if let Some(last_digest) = last_digest {
            debug!(
                "Last digest: {:?}, attestation_digest: {:?}",
                last_digest, attestation_digest
            );
            if last_digest == attestation_digest {
                debug!("Attestation confirmed on chain");
                return Ok(true);
            }
        }
        thread::sleep(duration);
    }

    Ok(false)
}
