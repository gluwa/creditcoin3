use anyhow::Result;
use diesel::dsl::exists as diesel_exists;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};

use attestor_primitives::AttestationCheckpoint as OnChainCheckpoint;

use super::schema::attestationcheckpoint::{
    self, dsl::attestationcheckpoint as attestation_checkpoint_table,
};

#[derive(Serialize, Deserialize, Debug, Insertable, Queryable, Selectable, Clone)]
#[diesel(table_name = attestationcheckpoint)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct AttestationCheckpoint {
    pub chain_id: i64,
    pub block_number: i64,
    pub digest: String,
    pub prev_digest: Option<String>,
}

impl AttestationCheckpoint {
    // Mapper from the OnChainCheckpoint to the db type
    pub fn from_on_chain(value: OnChainCheckpoint, chain_id: i64) -> Self {
        AttestationCheckpoint {
            chain_id,
            block_number: super::convert(value.block_number),
            digest: hex::encode(value.digest),
            prev_digest: value.prev_digest.map(hex::encode),
        }
    }
}

#[allow(dead_code)]
pub async fn get_by_digest(
    connection: &mut AsyncPgConnection,
    digest: String,
) -> Result<AttestationCheckpoint> {
    Ok(attestation_checkpoint_table
        .select(AttestationCheckpoint::as_select())
        .filter(attestationcheckpoint::digest.eq(digest))
        .first(connection)
        .await?)
}

#[allow(dead_code)]
pub async fn get_by_block_number(
    connection: &mut AsyncPgConnection,
    block_number: i64,
    chain_id: i64,
) -> Result<AttestationCheckpoint> {
    Ok(attestation_checkpoint_table
        .select(AttestationCheckpoint::as_select())
        .filter(attestationcheckpoint::block_number.eq(block_number))
        .filter(attestationcheckpoint::chain_id.eq(chain_id))
        .first(connection)
        .await?)
}

#[allow(dead_code)]
pub async fn exists_by_digest(connection: &mut AsyncPgConnection, digest: String) -> Result<bool> {
    Ok(diesel::select(diesel_exists(
        attestation_checkpoint_table
            .filter(attestationcheckpoint::digest.eq(digest.to_lowercase())),
    ))
    .get_result(connection)
    .await?)
}

#[allow(dead_code)]
pub async fn insert(
    connection: &mut AsyncPgConnection,
    checkpoint: AttestationCheckpoint,
) -> Result<()> {
    diesel::insert_into(attestation_checkpoint_table)
        .values(checkpoint)
        .execute(connection)
        .await?;

    Ok(())
}

#[allow(dead_code)]
pub async fn first_checkpoint_cached(
    connection: &mut AsyncPgConnection,
    chain_id: u64,
) -> Result<bool> {
    Ok(diesel::select(diesel_exists(
        attestation_checkpoint_table
            .filter(attestationcheckpoint::chain_id.eq(super::convert(chain_id)))
            .filter(attestationcheckpoint::prev_digest.is_null()),
    ))
    .get_result(connection)
    .await?)
}
