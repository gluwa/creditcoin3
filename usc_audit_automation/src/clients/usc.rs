use crate::clients::usc::decode::{
    decode_chain_key_dynamic, decode_checkpoint_dynamic, decode_interval_dynamic,
    decode_signed_attestation_dynamic, decode_static_or_dynamic, decode_supported_chain_dynamic,
    ContinuityProofStatus, DecodedSignedAttestation,
};
use anyhow::{Context, Result};
use attestor_primitives::{
    attestation_fragment::AttestationFragmentSerializable, AttestationCheckpoint, AttestationData,
    BlsSignature, Digest,
};
use parity_scale_codec::{Decode, Encode};
use scale_info::prelude::*;
use scale_info::TypeInfo;
use sp_core::H256;
use subxt::dynamic::{storage, Value};
use subxt::utils::AccountId32;
use subxt::{OnlineClient, PolkadotConfig};
use subxt_signer::{bip39::Mnemonic, sr25519::Keypair};
use tracing::info;
pub mod decode;
pub mod tests;

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, TypeInfo)]
pub struct SupportedChain {
    pub chain_id: u64,
    pub chain_name: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, TypeInfo)]
pub struct SignedAttestation<Digest, AccountId32> {
    pub attestation: AttestationData<Digest>,
    pub signature: BlsSignature,
    pub attestors: Vec<AccountId32>,
    pub continuity_proof: AttestationFragmentSerializable,
}

pub struct USCClient {
    api: OnlineClient<PolkadotConfig>,
    signer: Keypair,
    url: String,
}

impl USCClient {
    pub async fn new(url: impl Into<String>, mnemonic: &str) -> Result<Self> {
        let url = url.into();

        info!("Connecting to USC node at {url}");
        let api = OnlineClient::<PolkadotConfig>::from_url(&url).await?;

        // Create signer from mnemonic
        let mnemonic = Mnemonic::parse(mnemonic)?;
        let signer = Keypair::from_phrase(&mnemonic, None)
            .map_err(|e| anyhow::anyhow!("Invalid mnemonic: {e}"))?;

        Ok(Self { api, signer, url })
    }

    pub fn api(&self) -> &OnlineClient<PolkadotConfig> {
        &self.api
    }

    pub fn signer(&self) -> &Keypair {
        &self.signer
    }

    pub fn base_url(&self) -> &str {
        &self.url
    }

    pub async fn get_supported_chains(&self) -> anyhow::Result<Vec<SupportedChain>> {
        let mut supported = Vec::new();

        // Dynamic storage key prefix
        let prefix = storage("SupportedChains", "SupportedChains", vec![]);
        let mut iter = self.api.storage().at_latest().await?.iter(prefix).await?;

        while let Some(Ok(kv)) = iter.next().await {
            // Convert the thunk into a fully decoded Value
            if let Ok(val) = kv.value.to_value() {
                if let Some(decoded) = decode_supported_chain_dynamic(&val) {
                    supported.push(decoded);
                }
            }
        }

        Ok(supported)
    }
    pub async fn get_chain_key(&self, chain_id: u64, name: Vec<u8>) -> Result<Option<u64>> {
        let address = storage(
            "SupportedChains",
            "ChainIdAndNameToUniqKey",
            vec![Value::from(chain_id), Value::from(name)],
        );

        let maybe_val = self
            .api
            .storage()
            .at_latest()
            .await
            .context("Failed to access storage")?
            .fetch(&address)
            .await
            .context("Failed to fetch ChainIdAndNameToUniqKey")?;

        let chain_key = decode_static_or_dynamic(&maybe_val, |thunk| {
            let val = thunk.to_value().ok()?;
            decode_chain_key_dynamic(&val)
        })?;

        Ok(chain_key)
    }

    pub async fn fetch_last_digest(&self, chain_key: u64) -> anyhow::Result<Option<Digest>> {
        let address = storage("Attestation", "LastDigest", vec![Value::from(chain_key)]);

        let maybe_val = self
            .api
            .storage()
            .at_latest()
            .await?
            .fetch(&address)
            .await?;

        let result = maybe_val
            .map(|v| -> anyhow::Result<Digest> {
                let bytes = v.encoded();
                let digest = if bytes.len() == 32 {
                    // legacy: stored Digest directly
                    Digest::decode(&mut &bytes[..])?
                } else {
                    // current: stored (u64, Digest)
                    let (_block_number, digest) = <(u64, Digest)>::decode(&mut &bytes[..])?;
                    digest
                };
                Ok(digest)
            })
            .transpose()?;

        Ok(result)
    }

    pub async fn get_last_checkpoint(
        &self,
        chain_key: u64,
    ) -> anyhow::Result<Option<AttestationCheckpoint>> {
        let address = storage(
            "Attestation",
            "LastCheckpoint",
            vec![Value::from(chain_key)],
        );

        let maybe_val = self
            .api
            .storage()
            .at_latest()
            .await?
            .fetch(&address)
            .await
            .context("Failed to fetch Attestation.LastCheckpoint")?;

        let checkpoint = decode_static_or_dynamic(&maybe_val, |thunk| {
            let val = thunk.to_value().ok()?;
            decode_checkpoint_dynamic(&val)
        })?;

        Ok(checkpoint)
    }

    pub async fn get_attestation_by_digest(
        &self,
        chain_key: u64,
        digest: Digest,
    ) -> anyhow::Result<Option<DecodedSignedAttestation>> {
        let key_chain = Value::from(chain_key);
        let key_digest = Value::from_bytes(digest.as_bytes());
        let address = storage("Attestation", "Attestations", vec![key_chain, key_digest]);

        let maybe_val = self
            .api
            .storage()
            .at_latest()
            .await
            .context("Failed to access storage")?
            .fetch(&address)
            .await
            .with_context(|| format!("Failed to fetch attestation for chain key {chain_key}"))?;

        // Try to decode the SignedAttestation
        let result = if let Some(thunk) = maybe_val {
            // Try static decode first
            let bytes = thunk.encoded();
            match SignedAttestation::<H256, AccountId32>::decode(&mut &bytes[..]) {
                Ok(signed) => {
                    // Static decode succeeded - continuity_proof field was successfully decoded
                    // (even if blocks vec is empty, it decoded successfully)
                    Some(DecodedSignedAttestation {
                        value: signed,
                        proof_status: ContinuityProofStatus::Present,
                    })
                }
                Err(_) => {
                    // Static decode failed - fall back to dynamic decode which preserves proof_status
                    match thunk.to_value() {
                        Ok(val) => decode_signed_attestation_dynamic(&val),
                        Err(_) => None,
                    }
                }
            }
        } else {
            None
        };

        Ok(result)
    }

    pub async fn chain_checkpoint_interval(&self, chain_key: u64) -> anyhow::Result<Option<u32>> {
        // 1. Build dynamic address
        let address = storage(
            "Attestation",
            "AttestationCheckpointInterval",
            vec![Value::from(chain_key)],
        );

        // 2. Fetch from storage
        let maybe_val = self
            .api
            .storage()
            .at_latest()
            .await
            .context("Failed to access storage")?
            .fetch(&address)
            .await
            .context("Failed to fetch AttestationCheckpointInterval")?;

        // 3. Decode dynamically (correctly flatten Option)
        let interval = decode_static_or_dynamic(&maybe_val, |thunk| {
            let val = thunk.to_value().ok()?;
            decode_interval_dynamic(&val)
        })?;

        Ok(interval)
    }

    pub async fn chain_attestation_interval(&self, chain_key: u64) -> anyhow::Result<Option<u64>> {
        let address = storage(
            "Attestation",
            "ChainAttestationInterval",
            vec![Value::from(chain_key)],
        );

        let maybe_val = self
            .api
            .storage()
            .at_latest()
            .await?
            .fetch(&address)
            .await?;

        // Decode Option<u64>
        let interval = maybe_val
            .map(|v| -> anyhow::Result<u64> {
                let bytes = v.encoded();
                Ok(u64::decode(&mut &bytes[..])?)
            })
            .transpose()?;

        Ok(interval)
    }

    pub async fn get_attestation_vote_acceptance_window(
        &self,
        chain_key: u64,
    ) -> anyhow::Result<Option<u64>> {
        let address = storage(
            "Attestation",
            "VoteAcceptanceWindow",
            vec![Value::from(chain_key)],
        );

        let maybe_val = self
            .api
            .storage()
            .at_latest()
            .await?
            .fetch(&address)
            .await?;

        // Decode Option<u64>
        let window = maybe_val
            .map(|v| -> anyhow::Result<u64> {
                let bytes = v.encoded();
                Ok(u64::decode(&mut &bytes[..])?)
            })
            .transpose()?;

        Ok(window)
    }

    pub async fn get_attestation_header_by_digest(
        &self,
        chain_key: u64,
        digest: Digest,
    ) -> Result<Option<u64>> {
        let maybe_signed = self.get_attestation_by_digest(chain_key, digest).await?;
        // Map to the attestation's header number if present
        Ok(maybe_signed.map(|decoded_signed_attestation| {
            decoded_signed_attestation.value.attestation.header_number
        }))
    }
}
