use anyhow::Result;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};
use sp_core::H256;

use super::schema::cachedupto::onerow_id;
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
    pub onerow_id: bool,
    pub digest: String,
}

pub async fn mark_cached_up_to(connection: &mut AsyncPgConnection, digest: H256) -> Result<()> {
    let new_cached_through: CachedUpTo = digest.into();
    diesel::insert_into(cache_state_table)
        .values(&new_cached_through)
        .on_conflict(onerow_id)
        .do_update()
        .set(&new_cached_through)
        .execute(connection)
        .await?;

    Ok(())
}

pub async fn currently_cached_up_to(connection: &mut AsyncPgConnection) -> Option<CachedUpTo> {
    match cache_state_table
        .select(CachedUpTo::as_select())
        .first(connection)
        .await
    {
        Ok(entry) => Some(entry),
        Err(_e) => None,
    }
}

// Mapper from on-chain digest type (H256) to DB digest type (String)
impl From<H256> for CachedUpTo {
    fn from(digest: H256) -> Self {
        CachedUpTo {
            onerow_id: true,
            digest: hex::encode(digest),
        }
    }
}
