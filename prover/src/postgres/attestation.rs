use anyhow::Result;
use diesel::dsl::{exists as diesel_exists, select as diesel_select};
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};

use super::schema::attestation::{self, dsl::attestation as attestation_table};

#[derive(
    Serialize,
    Deserialize,
    Debug,
    Insertable,
    Queryable,
    Selectable,
    Clone,
    AsChangeset,
    PartialEq,
    Eq,
)]
#[diesel(table_name = attestation)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Attestation {
    pub chain_id: i64,
    pub header_number: i64,
    pub header_hash: String,
    pub tx_root: String,
    pub rx_root: String,
    pub digest: String,
}

pub async fn create_attestation(
    connection: &mut AsyncPgConnection,
    attestation: Attestation,
) -> Result<()> {
    diesel::insert_into(attestation_table)
        .values(attestation)
        .execute(connection)
        .await?;

    Ok(())
}

pub async fn exists_by_digest(connection: &mut AsyncPgConnection, digest: String) -> Result<bool> {
    Ok(diesel_select(diesel_exists(
        attestation_table.filter(attestation::digest.eq(digest)),
    ))
    .get_result(connection)
    .await?)
}

// Get Attestation for a range of header numbers
pub async fn get_attestation_range(
    connection: &mut AsyncPgConnection,
    chain_id: u64,
    start: i64,
    end: i64,
) -> Result<Vec<Attestation>> {
    Ok(attestation_table
        .filter(attestation::chain_id.eq(super::convert(chain_id)))
        .filter(attestation::header_number.ge(start))
        .filter(attestation::header_number.le(end))
        .select(Attestation::as_select())
        .load(connection)
        .await?)
}

// Upsert fragment
pub async fn upsert_attestation(
    connection: &mut AsyncPgConnection,
    attestations: &Vec<Attestation>,
) -> Result<()> {
    diesel::insert_into(attestation_table)
        .values(attestations)
        .on_conflict(attestation::digest)
        .do_nothing()
        .execute(connection)
        .await?;

    Ok(())
}
