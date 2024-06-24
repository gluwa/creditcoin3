use anyhow::Result;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};

use super::schema::signedattestation::{self, dsl::signedattestation as signedattestation_table};

//CREATE TABLE SignedAttestation (
//     id SERIAL PRIMARY KEY,
//     chain_id SMALLINT NOT NULL,
//     header_number BIGINT NOT NULL,
//     header_hash VARCHAR(64) NOT NULL,
//     tx_root VARCHAR(64) NOT NULL,
//     rx_root VARCHAR(64) NOT NULL,
//     digest VARCHAR(64) NOT NULL,
//     prev_digest VARCHAR(64),
//     signature VARCHAR(192) NOT NULL,
//     attestors TEXT [] NOT NULL
// );

#[derive(Serialize, Deserialize, Debug, Insertable, Queryable, Selectable)]
#[diesel(table_name = signedattestation)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Attestation {
    pub id: i32,
    pub chain_id: i16,
    pub header_number: i64,
    pub header_hash: String,
    pub tx_root: String,
    pub rx_root: String,
    pub digest: String,
    pub prev_digest: Option<String>,
    pub signature: String,
    pub attestors: Vec<Option<String>>,
}

pub async fn get_by_digest(
    connection: &mut AsyncPgConnection,
    digest: String,
) -> Result<Option<Attestation>> {
    Ok(Some(
        signedattestation_table
            .select(Attestation::as_select())
            .filter(signedattestation::digest.eq(digest))
            .first(connection)
            .await?,
    ))
}

pub async fn insert(connection: &mut AsyncPgConnection, attestation: Attestation) -> Result<()> {
    diesel::insert_into(signedattestation_table)
        .values(attestation)
        .execute(connection)
        .await?;

    Ok(())
}
