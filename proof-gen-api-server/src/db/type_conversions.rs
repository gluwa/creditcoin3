use anyhow::Result;
use chrono::NaiveDateTime;
use serde_json::Value;
use sp_core::H256;
use std::str::FromStr;
use tokio_postgres::Row;

use super::{ProofsDbEntry, QueryProofs};
use attestor_primitives::block::ContinuityProof;
use mmr::query_proof::QueryMerkleProof;

impl TryFrom<&Row> for ProofsDbEntry {
    type Error = anyhow::Error;

    fn try_from(row: &Row) -> Result<Self> {
        Ok(ProofsDbEntry {
            id: row.try_get("id")?,                       // SERIAL → i32
            chain_key: row.try_get("chain_key")?,         // BIGINT → i64
            header_number: row.try_get("header_number")?, // BIGINT → i64
            tx_index: row.try_get("tx_index")?,           // BIGINT → Option<i64>
            tx_hash: row.try_get("tx_hash")?,             // VARCHAR → Option<String>
            continuity_proof: row.try_get::<_, Option<Value>>("continuity_proof")?, // JSONB → Option<Value>
            merkle_proof: row.try_get::<_, Option<Value>>("merkle_proof")?, // JSONB → Option<Value>
            merkle_root: row.try_get("merkle_root")?, // VARCHAR → Option<String>
            created_at: row.try_get::<_, Option<NaiveDateTime>>("created_at")?,
            updated_at: row.try_get::<_, Option<NaiveDateTime>>("updated_at")?,
        })
    }
}

impl TryFrom<QueryProofs> for ProofsDbEntry {
    type Error = anyhow::Error;

    fn try_from(proofs: QueryProofs) -> Result<Self> {
        Ok(ProofsDbEntry {
            id: i32::default(), // Only actually used when fetching items from db, automatically assigned on insert
            chain_key: to_storage_int(proofs.chain_key),
            header_number: to_storage_int(proofs.header_number),
            tx_index: proofs.tx_index.map(to_storage_int),
            tx_hash: proofs.tx_hash.map(to_storage_hash),
            continuity_proof: proofs
                .continuity_proof
                .map(serde_json::to_value)
                .transpose()?,
            merkle_proof: proofs.merkle_proof.map(serde_json::to_value).transpose()?,
            merkle_root: proofs.merkle_root.map(to_storage_hash),
            created_at: Some(NaiveDateTime::default()), // Only used on read. Generated on insert
            updated_at: Some(NaiveDateTime::default()), // Only used on read. Generated on insert
        })
    }
}

impl TryFrom<ProofsDbEntry> for QueryProofs {
    type Error = anyhow::Error;

    fn try_from(entry: ProofsDbEntry) -> Result<Self> {
        Ok(QueryProofs {
            chain_key: from_storage_int(entry.chain_key),
            header_number: from_storage_int(entry.header_number),
            tx_index: entry.tx_index.map(from_storage_int),
            tx_hash: entry.tx_hash.map(|s| from_storage_hash(&s)),
            continuity_proof: entry
                .continuity_proof
                .map(serde_json::from_value::<ContinuityProof>)
                .transpose()?,
            merkle_proof: entry
                .merkle_proof
                .map(serde_json::from_value::<QueryMerkleProof>)
                .transpose()?,
            merkle_root: entry.merkle_root.map(|s| from_storage_hash(&s)),
        })
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
