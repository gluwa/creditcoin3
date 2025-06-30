use anyhow::Result;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use thiserror::Error;

use crate::postgres::schema::queryfragmenttype;
use crate::postgres::to_storage_type;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Database error: {0}")]
    DatabaseError(#[from] diesel::result::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, Queryable, AsChangeset)]
#[diesel(table_name = queryfragmenttype)]
pub struct QueryFragmentType {
    pub id: i32,
    pub query_id: String,
    pub chain_key: i64,
    pub height: i64,
    pub fragment_type: String,
    pub created_at: Option<SystemTime>,
    pub updated_at: Option<SystemTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Insertable)]
#[diesel(table_name = queryfragmenttype)]
pub struct NewQueryFragmentType {
    pub query_id: String,
    pub chain_key: i64,
    pub height: i64,
    pub fragment_type: String,
    pub created_at: Option<SystemTime>,
}

impl NewQueryFragmentType {
    pub fn new(query_id: String, chain_key: u64, height: u64, fragment_type: String) -> Self {
        Self {
            query_id,
            chain_key: to_storage_type(chain_key),
            height: to_storage_type(height),
            fragment_type,
            created_at: Some(SystemTime::now()),
        }
    }
}

pub async fn get_by_query_id(
    connection: &mut AsyncPgConnection,
    query_id_param: String,
) -> Result<Option<QueryFragmentType>, Error> {
    use crate::postgres::schema::queryfragmenttype::dsl::{query_id, queryfragmenttype};

    let result = queryfragmenttype
        .filter(query_id.eq(query_id_param))
        .first::<QueryFragmentType>(connection)
        .await;

    match result {
        Ok(qft) => Ok(Some(qft)),
        Err(diesel::NotFound) => Ok(None),
        Err(e) => Err(Error::DatabaseError(e)),
    }
}

pub async fn upsert(
    connection: &mut AsyncPgConnection,
    new_query_fragment_type: NewQueryFragmentType,
) -> Result<()> {
    use crate::postgres::schema::queryfragmenttype::dsl::{
        fragment_type, query_id, queryfragmenttype, updated_at,
    };

    diesel::insert_into(queryfragmenttype)
        .values(&new_query_fragment_type)
        .on_conflict(query_id)
        .do_update()
        .set((
            fragment_type.eq(&new_query_fragment_type.fragment_type),
            updated_at.eq(diesel::dsl::now),
        ))
        .execute(connection)
        .await?;

    Ok(())
}

pub async fn exists_by_query_id(
    connection: &mut AsyncPgConnection,
    query_id_param: String,
) -> Result<bool, Error> {
    use crate::postgres::schema::queryfragmenttype::dsl::{query_id, queryfragmenttype};
    use diesel::dsl::exists;
    use diesel::select;

    let result = select(exists(
        queryfragmenttype.filter(query_id.eq(query_id_param)),
    ))
    .get_result::<bool>(connection)
    .await?;

    Ok(result)
}
