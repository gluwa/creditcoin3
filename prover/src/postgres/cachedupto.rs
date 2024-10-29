use anyhow::Result;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};
use sp_core::H256;

use super::schema::cachedupto::chain_key as db_chain_key;
use super::schema::cachedupto::{self, dsl::cachedupto as cache_state_table};

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
#[diesel(table_name = cachedupto)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CachedUpTo {
    pub chain_key: i64,
    pub digest: String,
}

pub async fn mark_cached_up_to(
    connection: &mut AsyncPgConnection,
    chain_key: u64,
    digest: H256,
) -> Result<()> {
    let new_cached_through: CachedUpTo = (super::to_storage_type(chain_key), digest).into();
    diesel::insert_into(cache_state_table)
        .values(&new_cached_through)
        .on_conflict(db_chain_key)
        .do_update()
        .set(&new_cached_through)
        .execute(connection)
        .await?;

    Ok(())
}

pub async fn currently_cached_up_to(
    connection: &mut AsyncPgConnection,
    chain_key: u64,
) -> Option<CachedUpTo> {
    match cache_state_table
        .select(CachedUpTo::as_select())
        .filter(db_chain_key.eq(super::to_storage_type(chain_key)))
        .first(connection)
        .await
    {
        Ok(entry) => Some(entry),
        Err(_e) => None,
    }
}

// Mapper from chain key and on-chain digest (i64, H256) to DB type
impl From<(i64, H256)> for CachedUpTo {
    fn from(parts: (i64, H256)) -> Self {
        CachedUpTo {
            chain_key: parts.0,
            digest: hex::encode(parts.1),
        }
    }
}
