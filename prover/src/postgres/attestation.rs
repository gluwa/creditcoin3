use anyhow::Result;
use diesel::dsl::exists as diesel_exists;
use diesel::prelude::*;
use diesel::result::Error as DieselError;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};

use attestor_primitives::SignedAttestation;

use super::schema::attestation::{
    self, digest as SchemaDigest, dsl::attestation as attestation_table,
};

#[derive(Serialize, Deserialize, Debug, Insertable, Queryable, Selectable, Clone)]
#[diesel(table_name = attestation)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Attestation {
    pub chain_id: i64,
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
    chain_id: i64,
) -> Result<Attestation> {
    Ok(attestation_table
        .select(Attestation::as_select())
        .filter(attestation::header_number.eq(header_number))
        .filter(attestation::chain_id.eq(chain_id))
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

pub async fn insert(connection: &mut AsyncPgConnection, attestation: Attestation) -> Result<()> {
    diesel::insert_into(attestation_table)
        .values(attestation)
        .on_conflict(SchemaDigest)
        .do_nothing()
        .execute(connection)
        .await?;

    Ok(())
}

pub async fn first_digest_exists(
    connection: &mut AsyncPgConnection,
    chain_id: u64,
) -> Result<bool> {
    Ok(diesel::select(diesel_exists(
        attestation_table
            .filter(attestation::chain_id.eq(super::to_storage_type(chain_id)))
            .filter(attestation::prev_digest.is_null()),
    ))
    .get_result(connection)
    .await?)
}

pub async fn last_synced(
    connection: &mut AsyncPgConnection,
    chain_id: u64,
) -> Result<Option<Attestation>> {
    match attestation_table
        .order(attestation::header_number.asc())
        .filter(attestation::chain_id.eq(super::to_storage_type(chain_id)))
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

pub async fn remove_all_before(
    connection: &mut AsyncPgConnection,
    block_number: i64,
    chain_id: i64,
) -> Result<()> {
    let delete_target = attestation_table
        .filter(attestation::header_number.lt(block_number))
        .filter(attestation::chain_id.eq(chain_id));

    diesel::delete(delete_target).execute(connection).await?;

    Ok(())
}

// Mapper from the signed attestation to the db type
impl<H, A> From<SignedAttestation<H, A>> for Attestation
where
    H: AsRef<[u8]> + Clone + Copy,
    A: AsRef<[u8]> + Clone,
{
    fn from(value: SignedAttestation<H, A>) -> Self {
        Attestation {
            chain_id: super::to_storage_type(value.attestation.chain_id),
            header_number: super::to_storage_type(value.attestation.header_number),
            header_hash: hex::encode(value.attestation.header_hash),
            merkle_root: hex::encode(value.attestation.root),
            digest: hex::encode(value.digest()),
            prev_digest: value.attestation.prev_digest.map(hex::encode),
            signature: hex::encode(value.signature),
            attestors: value
                .attestors
                .iter()
                .map(|a| Some(hex::encode(a)))
                .collect(),
        }
    }
}
