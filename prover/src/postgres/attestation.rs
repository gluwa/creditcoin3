use anyhow::Result;
use diesel::{
    dsl::exists as diesel_exists, prelude::*, result::DatabaseErrorKind,
    result::Error as DieselError,
};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use hex::ToHex;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use attestor_primitives::SignedAttestation;

use super::schema::attestation::{
    self, digest as SchemaDigest, dsl::attestation as attestation_table,
    prev_digest as SchemaPrevDigest,
};

#[derive(Debug, Error)]
pub enum Error {
    #[error("Attempted to insert an attestation with a different digest, but duplicate chain key and block number. Clean DB and run prover to resync.")]
    DuplicateChainKeyAndBlockNumber,
    #[error("[0]")]
    Other(DieselError),
}

#[derive(Serialize, Deserialize, Debug, Insertable, Queryable, Selectable, Clone)]
#[diesel(table_name = attestation)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Attestation {
    pub chain_key: i64,
    pub header_number: i64,
    pub header_hash: String,
    pub merkle_root: String,
    pub digest: String,
    pub prev_digest: Option<String>,
    pub signature: String,
    pub attestors: Vec<Option<String>>,
}

pub async fn get_by_digest(
    connection: &mut AsyncPgConnection,
    digest: String,
) -> Result<Attestation> {
    Ok(attestation_table
        .select(Attestation::as_select())
        .filter(attestation::digest.eq(digest))
        .first(connection)
        .await?)
}

pub async fn get_by_header_number(
    connection: &mut AsyncPgConnection,
    header_number: i64,
    chain_key: i64,
) -> Result<Attestation> {
    Ok(attestation_table
        .select(Attestation::as_select())
        .filter(attestation::header_number.eq(header_number))
        .filter(attestation::chain_key.eq(chain_key))
        .first(connection)
        .await?)
}

pub async fn exists_by_digest(connection: &mut AsyncPgConnection, digest: String) -> Result<bool> {
    Ok(diesel::select(diesel_exists(
        attestation_table.filter(attestation::digest.eq(digest.to_lowercase())),
    ))
    .get_result(connection)
    .await?)
}

pub async fn insert(
    connection: &mut AsyncPgConnection,
    attestation: Attestation,
) -> Result<(), Error> {
    diesel::insert_into(attestation_table)
        .values(attestation)
        .on_conflict((SchemaDigest, SchemaPrevDigest))
        .do_nothing()
        .execute(connection)
        .await
        .map_err(|e| {
            if let DieselError::DatabaseError(DatabaseErrorKind::UniqueViolation, _) = e {
                // Only conflicts on (chain_key, block_number) are left unhandled
                Error::DuplicateChainKeyAndBlockNumber
            } else {
                Error::Other(e)
            }
        })?;

    Ok(())
}

pub async fn first_digest_exists(
    connection: &mut AsyncPgConnection,
    chain_key: u64,
) -> Result<bool> {
    Ok(diesel::select(diesel_exists(
        attestation_table
            .filter(attestation::chain_key.eq(super::to_storage_type(chain_key)))
            .filter(attestation::prev_digest.is_null()),
    ))
    .get_result(connection)
    .await?)
}

pub async fn last_synced(
    connection: &mut AsyncPgConnection,
    chain_key: u64,
) -> Result<Option<Attestation>> {
    match attestation_table
        .order(attestation::header_number.desc())
        .filter(attestation::chain_key.eq(super::to_storage_type(chain_key)))
        .select(Attestation::as_select())
        .first(connection)
        // Why does this not work?
        // .optional()
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

pub async fn earliest_attestation(
    connection: &mut AsyncPgConnection,
    chain_key: u64,
) -> Result<Option<Attestation>> {
    match attestation_table
        .order(attestation::header_number.asc())
        .filter(attestation::chain_key.eq(super::to_storage_type(chain_key)))
        .select(Attestation::as_select())
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

pub async fn remove_all_before(
    connection: &mut AsyncPgConnection,
    block_number: i64,
    chain_key: i64,
) -> Result<()> {
    let delete_target = attestation_table
        .filter(attestation::header_number.lt(block_number))
        .filter(attestation::chain_key.eq(chain_key));

    diesel::delete(delete_target).execute(connection).await?;

    Ok(())
}

/// Attestations equal to the claim block number are excluded via `.lt()`. This is because claims
/// in attestation blocks are considered to be at the end of the preceeding interval rather than
/// the start of the following one.
pub async fn get_highest_attestation_before(
    connection: &mut AsyncPgConnection,
    block_number: u64,
    chain_key: u64,
) -> Result<Option<Attestation>> {
    match attestation_table
        .order(attestation::header_number.desc())
        .filter(attestation::header_number.lt(super::to_storage_type(block_number)))
        .filter(attestation::chain_key.eq(super::to_storage_type(chain_key)))
        .select(Attestation::as_select())
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

/// Attestations equal to the claim block number are included via `.ge()`. This is because claims
/// in attestation blocks are considered to be at the end of the preceeding interval rather than
/// the start of the following one.
pub async fn get_lowest_attestation_after(
    connection: &mut AsyncPgConnection,
    block_number: u64,
    chain_key: u64,
) -> Result<Option<Attestation>> {
    match attestation_table
        .order(attestation::header_number.asc())
        .filter(attestation::header_number.ge(super::to_storage_type(block_number)))
        .filter(attestation::chain_key.eq(super::to_storage_type(chain_key)))
        .select(Attestation::as_select())
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

pub async fn get_highest_attestation(
    connection: &mut AsyncPgConnection,
    chain_key: u64,
) -> Result<Option<Attestation>> {
    match attestation_table
        .order(attestation::header_number.desc())
        .filter(attestation::chain_key.eq(super::to_storage_type(chain_key)))
        .select(Attestation::as_select())
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

// Mapper from the signed attestation to the db type
impl<H, A> From<SignedAttestation<H, A>> for Attestation
where
    H: AsRef<[u8]> + Clone + Copy,
    A: AsRef<[u8]> + Clone,
{
    fn from(value: SignedAttestation<H, A>) -> Self {
        Attestation {
            chain_key: super::to_storage_type(value.attestation.chain_key),
            header_number: super::to_storage_type(value.attestation.header_number),
            header_hash: hex::encode(value.attestation.header_hash),
            merkle_root: hex::encode(value.attestation.root),
            digest: value.digest().encode_hex(),
            prev_digest: value.attestation.prev_digest.map(|d| d.encode_hex()),
            signature: hex::encode(value.signature),
            attestors: value
                .attestors
                .iter()
                .map(|a| Some(hex::encode(a)))
                .collect(),
        }
    }
}
