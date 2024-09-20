use anyhow::Result;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};

use super::schema::fullycachedthrough::onerow_id;
use super::schema::fullycachedthrough::{self, dsl::fullycachedthrough as cache_state_table};

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
#[diesel(table_name = fullycachedthrough)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct FullyCachedThrough {
    pub onerow_id: bool,
    pub digest: String,
}

pub async fn mark_fully_cached_through(
    connection: &mut AsyncPgConnection,
    digest: String,
) -> Result<()> {
    let new_cached_through = FullyCachedThrough {
        onerow_id: true,
        digest,
    };

    diesel::insert_into(cache_state_table)
        .values(&new_cached_through)
        .on_conflict(onerow_id)
        .do_update()
        .set(&new_cached_through)
        .execute(connection)
        .await?;

    Ok(())
}

pub async fn currently_cached_through(connection: &mut AsyncPgConnection) -> Option<String> {
    match cache_state_table
        .select(FullyCachedThrough::as_select())
        .first(connection)
        .await
    {
        Ok(entry) => Some(entry.digest),
        Err(_e) => None,
    }
}
