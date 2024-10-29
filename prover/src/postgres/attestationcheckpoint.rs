use anyhow::Result;
use diesel::dsl::exists as diesel_exists;
use diesel::prelude::*;
use diesel::result::Error as DieselError;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use hex::ToHex;
use serde::{Deserialize, Serialize};

use attestor_primitives::AttestationCheckpoint as OnChainCheckpoint;

use super::schema::attestationcheckpoint::{
    self, digest as SchemaDigest, dsl::attestationcheckpoint as attestation_checkpoint_table,
};

#[derive(Serialize, Deserialize, Debug, Insertable, Queryable, Selectable, Clone)]
#[diesel(table_name = attestationcheckpoint)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct AttestationCheckpoint {
    pub chain_key: i64,
    pub block_number: i64,
    pub digest: String,
}

impl AttestationCheckpoint {
    // Mapper from the OnChainCheckpoint to the db type
    pub fn from_on_chain(value: &OnChainCheckpoint, chain_key: u64) -> Self {
        AttestationCheckpoint {
            chain_key: super::to_storage_type(chain_key),
            block_number: super::to_storage_type(value.block_number),
            digest: value.digest.encode_hex(),
        }
    }
}

pub async fn get_by_digest(
    connection: &mut AsyncPgConnection,
    digest: String,
) -> Result<AttestationCheckpoint> {
    Ok(attestation_checkpoint_table
        .select(AttestationCheckpoint::as_select())
        .filter(attestationcheckpoint::digest.eq(digest.to_lowercase()))
        .first(connection)
        .await?)
}

pub async fn get_by_block_number(
    connection: &mut AsyncPgConnection,
    block_number: i64,
    chain_key: i64,
) -> Result<AttestationCheckpoint> {
    Ok(attestation_checkpoint_table
        .select(AttestationCheckpoint::as_select())
        .filter(attestationcheckpoint::block_number.eq(block_number))
        .filter(attestationcheckpoint::chain_key.eq(chain_key))
        .first(connection)
        .await?)
}

pub async fn exists_by_digest(connection: &mut AsyncPgConnection, digest: String) -> Result<bool> {
    Ok(diesel::select(diesel_exists(
        attestation_checkpoint_table
            .filter(attestationcheckpoint::digest.eq(digest.to_lowercase())),
    ))
    .get_result(connection)
    .await?)
}

pub async fn insert(
    connection: &mut AsyncPgConnection,
    checkpoint: AttestationCheckpoint,
) -> Result<()> {
    diesel::insert_into(attestation_checkpoint_table)
        .values(checkpoint)
        .on_conflict(SchemaDigest)
        .do_nothing()
        .execute(connection)
        .await?;

    Ok(())
}

/// Checkpoints equal to the claim block number are excluded via `.lt()`. This is because claims
/// in checkpoint blocks are considered to be at the end of the preceeding interval rather than
/// the start of the following one.
pub async fn get_highest_checkpoint_before(
    connection: &mut AsyncPgConnection,
    block_number: u64,
    chain_key: u64,
) -> Result<Option<AttestationCheckpoint>> {
    match attestation_checkpoint_table
        .order(attestationcheckpoint::block_number.desc())
        .filter(attestationcheckpoint::block_number.lt(super::to_storage_type(block_number)))
        .filter(attestationcheckpoint::chain_key.eq(super::to_storage_type(chain_key)))
        .select(AttestationCheckpoint::as_select())
        .first(connection)
        .await
    {
        Ok(a) => Ok(Some(a)),
        Err(e) => {
            if e == DieselError::NotFound {
                Ok(None)
            } else {
                Err(e.into())
            }
        }
    }
}

/// Checkpoints equal to the claim block number are included via `.ge()`. This is because claims
/// in checkpoint blocks are considered to be at the end of the preceeding interval rather than
/// the start of the following one.
pub async fn get_lowest_checkpoint_after(
    connection: &mut AsyncPgConnection,
    block_number: u64,
    chain_key: u64,
) -> Result<Option<AttestationCheckpoint>> {
    match attestation_checkpoint_table
        .order(attestationcheckpoint::block_number.asc())
        .filter(attestationcheckpoint::block_number.ge(super::to_storage_type(block_number)))
        .filter(attestationcheckpoint::chain_key.eq(super::to_storage_type(chain_key)))
        .select(AttestationCheckpoint::as_select())
        .first(connection)
        .await
    {
        Ok(a) => Ok(Some(a)),
        Err(e) => {
            if e == DieselError::NotFound {
                Ok(None)
            } else {
                Err(e.into())
            }
        }
    }
}
