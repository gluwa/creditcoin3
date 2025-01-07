use anyhow::Result;
use bls_signatures::{PrivateKey, Serialize as BlsSerialize};
use serde::Serialize;
use sp_core::H256;
use thiserror::Error;
use tracing::{debug, error, info};

use cc_client::{AccountId32, Client as CcClient};

use attestor_primitives::{
    Attestation as AttestationPrimitive, AttestorId, BlsPublicKey, BlsSignature, ChainId, ChainKey,
    SignedAttestation, CHAIN_ID_TO_CHAIN_NAME,
};
use creditcoin3_attestor_gossip::communication::Attestation;
use vrf::ProofOfInclusion;

pub type Randomness = [u8; 32];

#[derive(Debug, Error)]
pub enum Error {
    #[error("Invalid BLS key")]
    InvalidBlsKey,
    #[error("Invalid proof of possession")]
    InvalidProofOfPossession,
    #[error("Failed to get chain key")]
    FailedToGetChainKey,
    #[error("Failed to get attestation interval")]
    FailedToGetAttestationInterval,
    #[error("Client error: {0}")]
    CclientError(#[from] cc_client::Error),
    #[error("Ethereum error: {0}")]
    EthError(#[from] eth::Error),
    #[error("Attestation fragment error: {0}")]
    FragmentError(#[from] attestation_chain::attestation_fragment::AttestationFragmentError),
    #[error("Attestation fragment block error: {0}")]
    FragmentBlockError(#[from] attestation_chain::block::BlockError),
    #[error("Failed to cast attestation_interval to usize: attestation_interval: {0}")]
    InvalidFragmentLength(u64),
    #[error("Trying to submit duplicate attestation")]
    DuplicateSubmission,
}

impl Error {
    #[must_use]
    pub fn is_not_selected_error(&self) -> bool {
        matches!(
            self,
            Error::CclientError(cc_client::Error::FailedToCreateProofOfInclusion(
                vrf::Error::NotSelected
            ))
        )
    }

    #[must_use]
    pub fn is_duplicate_submission(&self) -> bool {
        matches!(self, Error::DuplicateSubmission)
    }
}

#[derive(Debug, Clone, Serialize)]
struct SourceChainConfig {
    pub chain_key: ChainKey,
    pub current_attestation_interval: u64,
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
        self.chain_config.current_attestation_interval
    }

    #[must_use]
    pub fn get_chain_key(&self) -> ChainKey {
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
            .expect("Unknown chain id")
            .1;

        let chain_key = cc_client
            .get_chain_key(chain_id, chain_name.to_string())
            .await?
            .ok_or(Error::FailedToGetChainKey)?;

        let attestation_interval = cc_client
            .chain_attestation_interval(chain_key)
            .await?
            .ok_or(Error::FailedToGetAttestationInterval)?;

        let chain_config = SourceChainConfig {
            chain_key,
            current_attestation_interval: attestation_interval,
        };

        Ok(Self {
            cc_client,
            bls_keypair,
            chain_config,
        })
    }

    /// Init the client, this bootstraps registration if not registered already
    pub async fn init(&self) -> Result<()> {
        let is_attestor_member = self
            .cc_client
            .check_attestors_membership(self.get_chain_key())
            .await?;

        if !is_attestor_member {
            debug!("Signaling to start attesting... Please wait...");
            match self.start_attesting().await {
                Ok(()) => {
                    info!("Registration successful!");
                }
                Err(e) => {
                    if e.to_string().contains("Attestation::AddressNotAttestor") {
                        return Err(anyhow::anyhow!(
                            "The address is not an attestor. Please make sure the stash registers the attestor on chain first."
                        ));
                    }
                    error!("Failed to register: {:?}", e);
                    return Err(e);
                }
            }
        }

        info!("Attestator ready to start!");

        Ok(())
    }

    pub async fn can_attest(&self) -> Result<bool> {
        self.cc_client
            .check_attestors_membership(self.get_chain_key())
            .await
    }

    /// Register to the attestation pallet
    pub async fn start_attesting(&self) -> Result<()> {
        info!("Signaling intention to start attesting...");
        self.cc_client
            .start_attesting(
                self.get_chain_key(),
                self.get_bls_pubkey()?,
                self.proof_of_possession()?,
            )
            .await
    }

    pub async fn sign_attestation<H>(
        &self,
        attestation: AttestationPrimitive<H>,
    ) -> Result<Attestation<H, AttestorId>, Error>
    where
        H: Serialize + AsRef<[u8]> + Send + Sync + std::fmt::Debug + Clone,
    {
        let msg = attestation.serialize();
        // Sign the attestation data
        let signature = self.cc_client.sign(&msg);

        // sign attestation data with bls key
        let signature_bls = self.bls_keypair.sign(msg);

        // Sign the VRF output
        let vrf_output = self.sign_vrf(attestation.header_number).await?;

        // Create final attestation object
        Ok(Attestation {
            attestation_data: attestation,
            attestor: self.cc_client.get_attestor_id(),
            proof_of_inclusion: vrf_output,
            signature: sp_core::sr25519::Signature::from_raw(signature.0),
            signature_bls: attestor_primitives::bls::WrapEncode(signature_bls),
            continuity_proof: vec![],
        })
    }

    pub async fn sign_vrf(&self, header_number: u64) -> Result<ProofOfInclusion, Error> {
        let (randomness, epoch_index) =
            self.cc_client.fetch_babe_randomness_two_epoch_ego().await?;

        Ok(self
            .cc_client
            .sign_babe_vrf(self.get_chain_key(), header_number, randomness, epoch_index)
            .await?)
    }

    pub async fn get_last_attestation(
        &self,
        chain_key: ChainKey,
    ) -> Result<Option<SignedAttestation<H256, AccountId32>>, Error> {
        let last_digest = self.cc_client.fetch_last_digest(chain_key).await?;
        if let Some(digest) = last_digest {
            Ok(self
                .cc_client
                .get_attestation_by_digest(chain_key, digest)
                .await?)
        } else {
            Ok(None)
        }
    }

    pub async fn get_checkpoint_interval(&self) -> Result<u32, Error> {
        self.cc_client
            .chain_checkpoint_interval(self.get_chain_key())
            .await?
            .ok_or(Error::CclientError(
                cc_client::Error::NoCheckpointIntervalSet(self.get_chain_key()),
            ))
    }

    pub async fn submit_attestation<H>(
        &self,
        attestation: Attestation<H, AttestorId>,
    ) -> Result<(), Error>
    where
        H: Serialize + AsRef<[u8]> + Send + Sync + std::fmt::Debug + Clone,
    {
        Ok(self.cc_client.submit_attestation(attestation).await?)
    }

    pub fn change_attestation_interval(&mut self, new_interval: u64) {
        self.chain_config.current_attestation_interval = new_interval;
    }
}
