use super::QueryProofs;
use anyhow::Result;
use chrono::NaiveDateTime;
use serde_json::Value;
use sp_core::H256;
use std::str::FromStr;
use tokio_postgres::Row;

#[derive(Debug)]
pub struct ProofsDbEntry {
    pub id: i32,
    pub chain_key: i64,
    pub header_number: i64,
    pub tx_index: Option<i64>,
    pub tx_hash: Option<String>,
    pub continuity_proof: Option<Value>,
    pub merkle_proof: Option<Value>,
    pub merkle_root: Option<String>,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
}

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

impl From<QueryProofs> for ProofsDbEntry {
    fn from(proofs: QueryProofs) -> Self {
        ProofsDbEntry {
            id: i32::default(), // Only actually used when fetching items from db, automatically assigned on insert
            chain_key: to_storage_int(proofs.chain_key),
            header_number: to_storage_int(proofs.header_number),
            tx_index: proofs.tx_index.map(to_storage_int),
            tx_hash: proofs.tx_hash.map(|h| format!("{:#x}", h)),
            continuity_proof: proofs.continuity_proof,
            merkle_proof: proofs.merkle_proof,
            merkle_root: proofs.merkle_root.map(|h| format!("{:#x}", h)),
            created_at: Some(NaiveDateTime::default()), // Only used on read. Generated on insert
            updated_at: Some(NaiveDateTime::default()), // Only used on read. Generated on insert
        }
    }
}

impl Into<QueryProofs> for ProofsDbEntry {
    fn into(self) -> QueryProofs {
        QueryProofs {
            chain_key: from_storage_int(self.chain_key),
            header_number: from_storage_int(self.header_number),
            tx_index: self.tx_index.map(from_storage_int),
            tx_hash: self.tx_hash.map(|s| from_storage_hash(&s)),
            continuity_proof: None,
            merkle_proof: None,
            merkle_root: self.merkle_root.map(|s| from_storage_hash(&s)),
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
