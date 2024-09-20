use anyhow::Result;
use diesel::dsl::{exists as diesel_exists, select as diesel_select};
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};

use super::schema::blockwithdigest::{self, dsl::blockwithdigest as blocks_table};

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
#[diesel(table_name = blockwithdigest)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BlockWithDigest {
    pub chain_id: i64,
    pub header_number: i64,
    pub header_hash: String,
    pub merkle_root: String,
    pub digest: String,
}

pub async fn _create_attestation(
    connection: &mut AsyncPgConnection,
    block: BlockWithDigest,
) -> Result<()> {
    diesel::insert_into(blocks_table)
        .values(block)
        .execute(connection)
        .await?;

    Ok(())
}

pub async fn _exists_by_digest(connection: &mut AsyncPgConnection, digest: String) -> Result<bool> {
    Ok(diesel_select(diesel_exists(
        blocks_table.filter(blockwithdigest::digest.eq(digest)),
    ))
    .get_result(connection)
    .await?)
}

// Get Attestation for a range of header numbers
pub async fn get_blocks_in_range(
    connection: &mut AsyncPgConnection,
    chain_id: u64,
    start: i64,
    end: i64,
) -> Result<Vec<BlockWithDigest>> {
    Ok(blocks_table
        .filter(blockwithdigest::chain_id.eq(super::convert(chain_id)))
        .filter(blockwithdigest::header_number.ge(start))
        .filter(blockwithdigest::header_number.le(end))
        .select(BlockWithDigest::as_select())
        .load(connection)
        .await?)
}

// Upsert fragment
pub async fn upsert_fragment_blocks(
    connection: &mut AsyncPgConnection,
    blocks: &Vec<BlockWithDigest>,
) -> Result<()> {
    diesel::insert_into(blocks_table)
        .values(blocks)
        .on_conflict(blockwithdigest::digest)
        .do_nothing()
        .execute(connection)
        .await?;

    Ok(())
}
