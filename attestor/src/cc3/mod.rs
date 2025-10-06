use anyhow::Result;
use bls_signatures::{PrivateKey, Serialize as BlsSerialize};
use serde::Serialize;
use sp_core::H256;
use std::fmt;
use tracing::{debug, error, info};

use cc_client::{AccountId32, Client as CcClient};

use attestor_primitives::{
    attestation_fragment::AttestationFragmentSerializable, Attestation as AttestationPrimitive,
    AttestationCheckpoint, AttestorId, AttestorStatus, BlsPublicKey, BlsSignature, ChainId,
    ChainKey, SignedAttestation,
};
use creditcoin3_attestor_gossip::communication::Attestation;
use vrf::ProofOfInclusion;

use crate::error::Error;

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
    pub inner: CcClient,
    pub bls_keypair: PrivateKey,
    chain_config: SourceChainConfig,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("bls_keypair", &self.bls_keypair)
            .field("chain_config", &self.chain_config)
            .finish_non_exhaustive()
    }
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

impl Client {
    /// Create a new instance of cc3 client
    /// - `url`: rpc url of a creditcoin node
    /// - `key`: secret phrase for a creditcoin key
    /// - `chain_id`: chain id of the source chain
    pub async fn new(
        url: impl Into<String> + Clone,
        key: &str,
        chain_key: ChainKey,
        chain_id: ChainId,
    ) -> Result<Self, Error> {
        let cc_client = CcClient::new(url, key).await?;

        // Derive bls key from secret seed
        let bls_keypair = PrivateKey::new(key.as_bytes());

        let supported_chain = cc_client
            .get_supported_chain(chain_key)
            .await?
            .ok_or(Error::FailedToGetChainKey)?;

        if supported_chain.chain_id != chain_id {
            return Err(Error::WrongChain(chain_id, supported_chain.chain_id));
        }

        let chain_name = supported_chain.chain_name;
        info!(
            "⚙️ Configured attestor client for chain: {:?} with key {:?}",
            String::from_utf8(chain_name.clone()).map_err(|_| { Error::FailedToGetChainName })?,
            chain_key
        );

        let attestation_interval = cc_client
            .chain_attestation_interval(chain_key)
            .await?
            .ok_or(Error::FailedToGetAttestationInterval)?;

        let chain_config = SourceChainConfig {
            chain_key,
            current_attestation_interval: attestation_interval,
        };

        Ok(Self {
            inner: cc_client,
            bls_keypair,
            chain_config,
        })
    }

    /// Init the client, this bootstraps registration if not registered already
    pub async fn init(&self) -> Result<()> {
        // First check attestor public key to see if the attestor is registered
        let is_registered = self
            .inner
            .check_attestor_key_is_registered(self.get_chain_key())
            .await?;

        if is_registered {
            // Check attestor status
            let status = self.inner.get_attestor_status(self.get_chain_key()).await?;
            match status {
                Some(AttestorStatus::Active) => {
                    debug!("Attestor is already active and ready to attest");
                }
                Some(AttestorStatus::Idle) => {
                    debug!(
                        "Attestor is in idle state, signaling to start attesting... Please wait..."
                    );
                    match self.start_attesting().await {
                        Ok(()) => {
                            info!("Successfully signaled intention to attest!");
                        }
                        Err(e) => {
                            error!("Failed to signal intention to attest: {:?}", e);
                            return Err(e);
                        }
                    }
                }
                Some(AttestorStatus::Waiting) => {
                    debug!("Attestor is waiting to be elected, no action needed");
                }
                _ => {
                    return Err(anyhow::anyhow!(
                        "Unknown attestor status or attestor not found"
                    ));
                }
            }
        } else {
            // Check didn't early exit, meaning the attestor is registered on chain, it's just missing the key
            debug!("Attestor bls key not registered yet, signaling to start attesting... Please wait...");
            match self.start_attesting().await {
                Ok(()) => {
                    info!("Registration successful!");
                }
                Err(e) => {
                    // Just in case
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

        debug!("Attestor ready to start!");

        Ok(())
    }

    pub async fn can_attest(&self) -> Result<bool> {
        // Check if attestor is in active set (elected)
        let is_a_member = self
            .inner
            .check_attestors_membership(self.get_chain_key())
            .await?;

        if is_a_member {
            return Ok(true);
        }

        // If not active, check if attestor is registered and waiting
        let is_key_registered = self
            .inner
            .check_attestor_key_is_registered(self.get_chain_key())
            .await?;

        // If we came to this point, it means that the check didn't early exit and the attestor is
        // registered, it's just missing the key i.e. attest() hasn't been called yet
        if !is_key_registered {
            return Ok(false);
        }

        // Check the attestor status
        let status = self.inner.get_attestor_status(self.get_chain_key()).await?;
        match status {
            Some(AttestorStatus::Active) => Ok(true),
            _ => Ok(false),
        }
    }

    /// Register to the attestation pallet
    pub async fn start_attesting(&self) -> Result<()> {
        info!("Signaling intention to start attesting...");
        self.inner
            .start_attesting(
                self.get_chain_key(),
                self.get_bls_pubkey()?,
                self.proof_of_possession()?,
            )
            .await
    }

    pub fn sign_attestation<H>(
        &self,
        attestation: AttestationPrimitive<H>,
        continuity_proof: AttestationFragmentSerializable,
        vrf_output: ProofOfInclusion,
        epoch: u64,
    ) -> Attestation<H, AttestorId>
    where
        H: Serialize + AsRef<[u8]> + Send + Sync + std::fmt::Debug + Clone,
    {
        let msg = attestation.serialize();
        // Sign the attestation data
        let signature = self.inner.sign(&msg);

        // sign attestation data with bls key
        let signature_bls = self.bls_keypair.sign(msg);

        // Create final attestation object
        Attestation {
            attestation_data: attestation,
            attestor: self.inner.get_attestor_id(),
            proof_of_inclusion: vrf_output,
            signature: sp_core::sr25519::Signature::from_raw(signature.0),
            signature_bls: attestor_primitives::bls::WrapEncode(signature_bls),
            continuity_proof,
            epoch,
        }
    }

    pub async fn sign_vrf(&self, header_number: u64) -> Result<ProofOfInclusion, Error> {
        let (randomness, epoch_index) = self.inner.fetch_babe_randomness_two_epoch_ego().await?;

        Ok(self
            .inner
            .sign_babe_vrf(self.get_chain_key(), header_number, randomness, epoch_index)
            .await?)
    }

    pub async fn get_last_attestation(
        &self,
        chain_key: ChainKey,
    ) -> Result<Option<SignedAttestation<H256, AccountId32>>, Error> {
        let last_digest = self.inner.fetch_last_digest(chain_key).await?;
        if let Some(digest) = last_digest {
            Ok(self
                .inner
                .get_attestation_by_digest(chain_key, digest)
                .await?)
        } else {
            Ok(None)
        }
    }

    pub async fn get_last_checkpoint(
        &self,
        chain_key: ChainKey,
    ) -> Result<Option<AttestationCheckpoint>, Error> {
        Ok(self.inner.get_last_checkpoint(chain_key).await?)
    }

    pub async fn get_checkpoint_interval(&self) -> Result<u32, Error> {
        self.inner
            .chain_checkpoint_interval(self.get_chain_key())
            .await?
            .ok_or(Error::Cclient(cc_client::Error::NoCheckpointIntervalSet(
                self.get_chain_key(),
            )))
    }

    pub async fn get_current_epoch(&self) -> Result<u64> {
        Ok(self.inner.get_current_epoch().await?)
    }

    pub async fn submit_attestation<H>(
        &self,
        attestation: Attestation<H, AttestorId>,
    ) -> Result<(), Error>
    where
        H: Serialize + AsRef<[u8]> + Send + Sync + std::fmt::Debug + Clone,
    {
        Ok(self.inner.submit_attestation(attestation).await?)
    }

    pub fn change_attestation_interval(&mut self, new_interval: u64) {
        self.chain_config.current_attestation_interval = new_interval;
    }

    pub async fn get_attestation_chain_genesis_block_number(&self) -> Result<u64, Error> {
        Ok(self
            .inner
            .get_attestation_chain_genesis_block_number(self.get_chain_key())
            .await?)
    }

    pub async fn get_vote_acceptance_window(&self, chain_key: ChainKey) -> Result<u64, Error> {
        self.inner
            .get_attestation_vote_acceptance_window(chain_key)
            .await?
            .ok_or(Error::Cclient(cc_client::Error::NoVoteAcceptanceWindowSet(
                self.get_chain_key(),
            )))
    }
}
