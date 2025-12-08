use anyhow::Result;
use chrono::Utc;
use hex;
use sp_core::H256;
use std::str::FromStr;

use crate::{
    db::{
        continuity_proofs::{
            ContinuityProofInsertable, ContinuityProofItem, ContinuityProofRecord,
        },
        merkle_proofs::{MerkleProofInsertable, MerkleProofItem, MerkleProofRecord},
    },
    services::continuity_service::ContinuityResponse,
};
use attestor_primitives::block::ContinuityProof;
use merkle::proof::TransactionMerkleProof;

impl TryFrom<MerkleProofItem> for MerkleProofInsertable {
    type Error = anyhow::Error;

    fn try_from(proof: MerkleProofItem) -> Result<Self> {
        Ok(MerkleProofInsertable {
            chain_key: to_storage_int(proof.chain_key),
            header_number: to_storage_int(proof.header_number),
            tx_index: proof.tx_index.map(to_storage_int),
            tx_hash: proof.tx_hash.map(to_storage_hash),
            tx_bytes: proof.tx_bytes,
            merkle_proof: serde_json::to_value(proof.merkle_proof)?,
            merkle_root: to_storage_hash(proof.merkle_root),
        })
    }
}

impl TryFrom<MerkleProofRecord> for MerkleProofItem {
    type Error = anyhow::Error;

    fn try_from(entry: MerkleProofRecord) -> Result<Self> {
        Ok(MerkleProofItem {
            chain_key: from_storage_int(entry.chain_key),
            header_number: from_storage_int(entry.header_number),
            tx_index: entry.tx_index.map(from_storage_int),
            tx_hash: entry.tx_hash.map(|s| from_storage_hash(&s)),
            tx_bytes: entry.tx_bytes,
            merkle_proof: serde_json::from_value::<TransactionMerkleProof>(entry.merkle_proof)?,
            merkle_root: from_storage_hash(&entry.merkle_root),
        })
    }
}

impl TryFrom<ContinuityProofItem> for ContinuityProofInsertable {
    type Error = anyhow::Error;

    fn try_from(cont_proof: ContinuityProofItem) -> Result<Self> {
        Ok(ContinuityProofInsertable {
            chain_key: to_storage_int(cont_proof.chain_key),
            header_number: to_storage_int(cont_proof.header_number),
            continuity_proof: serde_json::to_value(cont_proof.continuity_proof)?,
        })
    }
}

impl TryFrom<ContinuityProofRecord> for ContinuityProofItem {
    type Error = anyhow::Error;

    fn try_from(entry: ContinuityProofRecord) -> Result<Self> {
        Ok(ContinuityProofItem {
            chain_key: from_storage_int(entry.chain_key),
            header_number: from_storage_int(entry.header_number),
            continuity_proof: serde_json::from_value::<ContinuityProof>(entry.continuity_proof)?,
        })
    }
}

impl From<(MerkleProofItem, ContinuityProofItem)> for ContinuityResponse {
    fn from(proofs: (MerkleProofItem, ContinuityProofItem)) -> Self {
        let (merkle, continuity) = proofs;
        // Convert tx_bytes
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

#[must_use]
pub fn to_storage_int(num: u64) -> i64 {
    i64::from_ne_bytes(num.to_ne_bytes())
}

#[must_use]
pub fn from_storage_int(num: i64) -> u64 {
    u64::from_ne_bytes(num.to_ne_bytes())
}

#[must_use]
pub fn to_storage_hash(hash: H256) -> String {
    format!("{hash:#x}")
}

/// @param hash: A 0x prefixed hex representation of a hash
#[must_use]
pub fn from_storage_hash(hash: &str) -> H256 {
    match H256::from_str(hash) {
        Ok(hash) => hash,
        Err(e) => {
            panic!("ProofsDbEntry failed to convert to QueryProofs. This shouldn't fail gracefully. Error: {e}, Hash: {hash}")
        }
    }
}
