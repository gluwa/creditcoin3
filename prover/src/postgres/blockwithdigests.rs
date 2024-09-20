use anyhow::Result;
use diesel::dsl::{exists as diesel_exists, select as diesel_select};
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};

use super::schema::blockwithdigests::{self, dsl::blockwithdigests as blocks_table};

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
#[diesel(table_name = blockwithdigests)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BlockWithDigests {
    pub chain_id: i64,
    pub header_number: i64,
    pub header_hash: String,
    pub merkle_root: String,
    pub digest: String,
}

pub async fn _create_attestation(
    connection: &mut AsyncPgConnection,
    block: BlockWithDigests,
) -> Result<()> {
    diesel::insert_into(blocks_table)
        .values(block)
        .execute(connection)
        .await?;

    Ok(())
}

pub async fn _exists_by_digest(connection: &mut AsyncPgConnection, digest: String) -> Result<bool> {
    Ok(diesel_select(diesel_exists(
        blocks_table.filter(blockwithdigests::digest.eq(digest)),
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
) -> Result<Vec<BlockWithDigests>> {
    Ok(blocks_table
        .filter(blockwithdigests::chain_id.eq(super::convert(chain_id)))
        .filter(blockwithdigests::header_number.ge(start))
        .filter(blockwithdigests::header_number.le(end))
        .select(BlockWithDigests::as_select())
        .load(connection)
        .await?)
}

// Upsert fragment
pub async fn upsert_attestation(
    connection: &mut AsyncPgConnection,
    blocks: &Vec<BlockWithDigests>,
) -> Result<()> {
    diesel::insert_into(blocks_table)
        .values(blocks)
        .on_conflict(blockwithdigests::digest)
        .do_nothing()
        .execute(connection)
        .await?;

    Ok(())
}
