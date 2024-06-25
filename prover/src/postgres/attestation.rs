use anyhow::Result;
use diesel::dsl::exists as diesel_exists;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};

use attestor_primitives::SignedAttestation;

use super::schema::signedattestation::{self, dsl::signedattestation as signedattestation_table};

#[derive(Serialize, Deserialize, Debug, Insertable, Queryable, Selectable)]
#[diesel(table_name = signedattestation)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Attestation {
    pub chain_id: i64,
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
            .await
            .map_err(|e| {
                tracing::error!("Error getting attestation by digest: {:?}", e);
                anyhow::anyhow!(e)
            })?,
    ))
}

pub async fn exists_by_digest(connection: &mut AsyncPgConnection, digest: String) -> Result<bool> {
    Ok(diesel::select(diesel_exists(
        signedattestation_table.filter(signedattestation::digest.eq(digest.to_lowercase())),
    ))
    .get_result(connection)
    .await?)
}

pub async fn insert(connection: &mut AsyncPgConnection, attestation: Attestation) -> Result<()> {
    diesel::insert_into(signedattestation_table)
        .values(attestation)
        .execute(connection)
        .await
        .map_err(|e| {
            tracing::error!("Error inserting attestation: {:?}", e);
            anyhow::anyhow!(e)
        })?;

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
            chain_id: value.attestation.chain_id as i64,
            header_number: value.attestation.header_number as i64,
            header_hash: hex::encode(value.attestation.header_hash),
            tx_root: hex::encode(value.attestation.tx_root),
            rx_root: hex::encode(value.attestation.rx_root),
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
