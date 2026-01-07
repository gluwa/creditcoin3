use anyhow::Result;
use bigdecimal::{BigDecimal, ToPrimitive};
use chrono::Utc;
use hex;
use sp_core::H256;
use std::str::FromStr;

use crate::{
    db::continuity_proofs::{
        ContinuityProofInsertable, ContinuityProofItem, ContinuityProofRecord,
    },
    services::continuity_service::{ContinuityResponse, MerkleProofItem},
};
use attestor_primitives::block::ContinuityProof;

impl TryFrom<ContinuityProofItem> for ContinuityProofInsertable {
    type Error = anyhow::Error;

    fn try_from(cont_proof: ContinuityProofItem) -> Result<Self> {
        Ok(ContinuityProofInsertable {
            chain_key: cont_proof.chain_key as i64,
            header_number: BigDecimal::from(cont_proof.header_number),
            continuity_proof: serde_json::to_value(cont_proof.continuity_proof)?,
            ends_in_attestation: cont_proof.ends_in_attestation,
        })
    }
}

impl TryFrom<ContinuityProofRecord> for ContinuityProofItem {
    type Error = anyhow::Error;

    fn try_from(entry: ContinuityProofRecord) -> Result<Self> {
        let header_number = entry
            .header_number
            .to_u64()
            .ok_or_else(|| anyhow::anyhow!("header_number out of u64 range"))?;

        Ok(ContinuityProofItem {
            chain_key: entry.chain_key as u64,
            header_number,
            continuity_proof: serde_json::from_value::<ContinuityProof>(entry.continuity_proof)?,
            ends_in_attestation: entry.ends_in_attestation,
        })
    }
}

impl From<(MerkleProofItem, ContinuityProofItem)> for ContinuityResponse {
    fn from(proofs: (MerkleProofItem, ContinuityProofItem)) -> Self {
        let (merkle, continuity) = proofs;
        // Convert tx_bytes to hex
        let tx_bytes_hex = merkle
            .tx_bytes
            .map(|bytes| format!("0x{}", hex::encode(&bytes)));
        ContinuityResponse {
            chain_key: merkle.chain_key,
            header_number: merkle.header_number,
            tx_index: merkle.tx_index,
            tx_hash: merkle.tx_hash.map(|h| format!("0x{h:x}")),
            tx_bytes: tx_bytes_hex,
            continuity_proof: continuity.continuity_proof,
            merkle_proof: Some(merkle.merkle_proof),
            cached: true,
            generated_at: Utc::now(), // Maybe retain created_at and fill here
        }
    }
}

// TODO: Use this for attestation storage in future pr
#[allow(unused)]
pub(crate) fn to_storage_hash(hash: H256) -> String {
    format!("{hash:#x}")
}

// TODO: Use this for attestation storage in future pr
#[allow(unused)]
/// @param hash: A 0x prefixed hex representation of a hash
pub(crate) fn from_storage_hash(hash: &str) -> H256 {
    match H256::from_str(hash) {
        Ok(hash) => hash,
        Err(e) => {
            panic!("ProofsDbEntry failed to convert to QueryProofs. This shouldn't fail gracefully. Error: {e}, Hash: {hash}")
        }
    }
}
