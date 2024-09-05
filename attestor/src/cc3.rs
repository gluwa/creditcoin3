use anyhow::Result;
use bls_signatures::{PrivateKey, Serialize as BlsSerialize};
use exponential_backoff::Backoff;
use kameo::{
    actor::Actor,
    message::{Context, Message},
};
use serde::Serialize;
use sp_core::H256;
use std::{thread, time::Duration};
use tracing::{debug, error, info, warn};

use cc_client::Client as CcClient;
pub use cc_client::Error;

use attestor_primitives::{
    Attestation as AttestationPrimitive, AttestorId, BlsPublicKey, BlsSignature, ChainId,
};
use creditcoin3_attestor_gossip::{Attestation, Topic};

pub type Randomness = [u8; 32];

// Chain id to chain name mapping
// Only these are supported for now
const CHAIN_ID_TO_CHAIN_NAME: [(u64, &str); 3] = [
    (1, "Ethereum"),
    (31337, "Local anvil"),
    (11_155_111, "Sepolia ethereum"),
];

#[derive(Debug, Clone, Serialize)]
struct SourceChainConfig {
    pub chain_key: ChainId,
    pub attestation_interval: u64,
}

#[derive(Clone)]
/// Cc3 client that is configured with an url and keypair
/// Must connect to a node that has rpc and websocket enabled
/// - `cc_client`: Creditcoin3 client
/// - `bls_keypair`: BLS keypair
pub struct Client {
    pub cc_client: CcClient,
    pub bls_keypair: PrivateKey,
    chain_config: SourceChainConfig,
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

    #[must_use]
    pub fn get_attestation_interval(&self) -> u64 {
        self.chain_config.attestation_interval
    }

    #[must_use]
    pub fn get_chain_key(&self) -> ChainId {
        self.chain_config.chain_key
    }
}

impl<'a> Client {
    /// Create a new instance of cc3 client
    /// - `url`: rpc url of a creditcoin node
    /// - `key`: secret phrase for a creditcoin key
    /// - `chain_id`: chain id of the source chain
    pub async fn new(
        url: impl Into<String> + Clone,
        key: &'a str,
        chain_id: ChainId,
    ) -> Result<Self> {
        let cc_client = CcClient::new(url, key).await?;

        // Derive bls key from secret seed
        let bls_keypair = PrivateKey::new(key.as_bytes());

        let chain_name = CHAIN_ID_TO_CHAIN_NAME
            .iter()
            .find(|(id, _)| *id == chain_id)
            .expect("Unknown chain id");

        let chain_key = cc_client
            .get_chain_key(chain_id, chain_name.1.to_string())
            .await?
            .ok_or(Error::FailedToGetChainKey)?;

        let attestation_interval = cc_client
            .chain_attestation_interval(chain_key)
            .await?
            .ok_or(Error::FailedToGetAttestationInterval)?;

        let chain_config = SourceChainConfig {
            chain_key,
            attestation_interval,
        };

        Ok(Self {
            cc_client,
            bls_keypair,
            chain_config,
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
        let vrf_output = self
            .cc_client
            .sign_babe_vrf(attestation.header_number)
            .await
            .map_err(|e| {
                error!("Error signing babe vrf: {:?}", e);
                Error::FailedToSignBabeVrf
            })?;

        info!("attestation to submit: {:?}", attestation);

        // Create final attestation object
        Ok(Attestation {
            attestation_data: attestation,
            attestor: self.cc_client.get_attestor_id(),
            topic: Topic::new(1),
            proof_of_inclusion: vrf_output,
            signature: sp_core::sr25519::Signature::from_raw(signature.0),
            signature_bls: attestor_primitives::bls::WrapEncode(signature_bls),
        })
    }

    pub async fn submit_attestation<H>(
        &self,
        mut attestation: AttestationPrimitive<H>,
    ) -> Result<()>
    where
        H: Serialize + AsRef<[u8]> + Send + Sync + std::fmt::Debug + Clone,
    {
        // We need to override the chain_id with the one we are attesting to
        // because we have a different mapping in cc next
        attestation.chain_id = self.chain_config.chain_key;
        let chain_id = attestation.chain_id;

        if attestation.header_number % self.chain_config.attestation_interval != 0 {
            warn!(
                "Skipping Attestation because it's not in the configured interval for this chain"
            );
            return Ok(());
        };

        let is_attestor_member = self.cc_client.check_attestors_membership().await?;
        if !is_attestor_member {
            warn!("Attestor is not valid at current timeframe, skipping...");
            return Ok(());
        };

        // Get the digest of the attestation
        let attestation_digest = attestation.digest();

        // check if attestation already exists
        // if yes, don't submit
        let exists = self
            .cc_client
            .chain_attestation_exists(chain_id, attestation_digest)
            .await?;

        if exists {
            warn!("Attestation already exists, skipping...");
            return Ok(());
        }

        // Get the last digest from the chain
        // and set it as the previous digest of the attestation
        let prev_digest = self.cc_client.fetch_last_digest(chain_id).await?;
        attestation.prev_digest = prev_digest;

        let mut inclusion = false;
        while !inclusion {
            info!("Trying to submit attestation...");
            let attestation = self
                .sign_attestation(attestation.clone())
                .await
                .map_err(|e| {
                    error!("Error signing attestation: {:?}", e);
                    Error::FailedToSignBabeVrf
                })?;

            // Submit the attestation to the chain
            self.cc_client
                .submit_attestation(attestation)
                .await
                .map_err(|e| {
                    error!("Error submitting attestation: {:?}", e);
                    Error::FailedToSubmit
                })?;

            inclusion =
                check_attestation_inclusion(self.cc_client.clone(), chain_id, attestation_digest)
                    .await?;
        }

        info!("✅ Attestation with digest {attestation_digest} included in chain");

        Ok(())
    }
}

impl Actor for Client {}

// AttestationSubmit is a message that can be sent to submit an attestation over rpc to the cc3 node
// It holds the attestation data to be signed by the attestor before submitting
pub struct AttestationSubmit<H> {
    pub attestation: Option<AttestationPrimitive<H>>,
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

        match self.submit_attestation(msg.attestation.unwrap()).await {
            Ok(()) => {
                info!("Attestation submitted successfully");
            }
            Err(e) => {
                error!("Error submitting attestation: {:?}", e);
            }
        }

        Ok(())
    }
}

/// Check if the attestation is included in the chain
/// - `cc_client`: Creditcoin3 client
/// - `chain_id`: Chain id
/// - `attestation_digest`: Attestation digest
/// Returns a boolean indicating if the attestation is included in the chain
/// It retries 4 times with 6 seconds interval
pub async fn check_attestation_inclusion(
    cc_client: CcClient,
    chain_id: ChainId,
    attestation_digest: H256,
) -> Result<bool> {
    let retries = 4;
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
