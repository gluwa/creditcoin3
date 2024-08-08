use anyhow::Result;
use diesel::dsl::exists as diesel_exists;
use diesel::prelude::*;
use diesel::result::Error as DieselError;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};

use attestor_primitives::SignedAttestation;

use super::schema::attestationcheckpoint::{
    self, dsl::attestationcheckpoint as attestation_checkpoint_table,
};

#[derive(Serialize, Deserialize, Debug, Insertable, Queryable, Selectable, Clone)]
#[diesel(table_name = attestationcheckpoint)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct AttestationCheckpoint {
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
) -> Result<AttestationCheckpoint> {
    Ok(attestation_checkpoint_table
        .select(AttestationCheckpoint::as_select())
        .filter(attestationcheckpoint::digest.eq(digest))
        .first(connection)
        .await?)
}

pub async fn get_by_header_number(
    connection: &mut AsyncPgConnection,
    header_number: i64,
    chain_id: i64,
) -> Result<AttestationCheckpoint> {
    Ok(attestation_checkpoint_table
        .select(AttestationCheckpoint::as_select())
        .filter(attestationcheckpoint::header_number.eq(header_number))
        .filter(attestationcheckpoint::chain_id.eq(chain_id))
        .first(connection)
        .await?)
}

pub async fn exists_by_digest(connection: &mut AsyncPgConnection, digest: String) -> Result<bool> {
    Ok(diesel::select(diesel_exists(
        attestation_checkpoint_table
            .filter(attestationcheckpoint::digest.eq(digest.to_lowercase())),
    ))
    .get_result(connection)
    .await?)
}

pub async fn insert(
    connection: &mut AsyncPgConnection,
    attestation: AttestationCheckpoint,
) -> Result<()> {
    diesel::insert_into(attestation_checkpoint_table)
        .values(attestation)
        .execute(connection)
        .await?;

    Ok(())
}

pub async fn first_digest_exists(
    connection: &mut AsyncPgConnection,
    chain_id: u64,
) -> Result<bool> {
    Ok(diesel::select(diesel_exists(
        attestation_checkpoint_table
            .filter(attestationcheckpoint::chain_id.eq(super::convert(chain_id)))
            .filter(attestationcheckpoint::prev_digest.is_null()),
    ))
    .get_result(connection)
    .await?)
}

pub async fn last_synced(
    connection: &mut AsyncPgConnection,
    chain_id: u64,
) -> Result<Option<AttestationCheckpoint>> {
    match attestation_checkpoint_table
        .order(attestationcheckpoint::header_number.asc())
        .filter(attestationcheckpoint::chain_id.eq(super::convert(chain_id)))
        .select(AttestationCheckpoint::as_select())
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

// Mapper from the signed attestation to the db type
impl<H, A> From<SignedAttestation<H, A>> for AttestationCheckpoint
where
    H: AsRef<[u8]> + Clone + Copy,
    A: AsRef<[u8]> + Clone,
{
    fn from(value: SignedAttestation<H, A>) -> Self {
        AttestationCheckpoint {
            chain_id: super::convert(value.attestation.chain_id),
            header_number: super::convert(value.attestation.header_number),
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
