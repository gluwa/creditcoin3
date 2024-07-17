use anyhow::Result;
use diesel::dsl::exists as diesel_exists;
use diesel::prelude::*;
use diesel::result::Error as DieselError;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};

use attestor_primitives::SignedAttestation;

use super::schema::signedattestation::{self, dsl::signedattestation as signedattestation_table};
use attestation_chain::block::Block;
use starknet_types_core::felt::Felt as FieldElement;
use tracing::info;

#[derive(Serialize, Deserialize, Debug, Insertable, Queryable, Selectable, Clone)]
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

impl From<Attestation> for Block {
    fn from(attestation: Attestation) -> Self {
        info!("Converting attestation to block: {:?}", attestation);
        info!("tx_root : {:?}", attestation.tx_root);
        info!("tx_root str: {:?}", attestation.tx_root.as_str());

        Block {
            block_number: attestation.header_number.into(),
            tx_root: hex_to_felt(&attestation.tx_root).unwrap(),
            rx_root: hex_to_felt(&attestation.rx_root).unwrap(),
            prev_digest: FieldElement::from_dec_str(&attestation.prev_digest.expect("Some"))
                .unwrap(),
            digest: FieldElement::from_dec_str(&attestation.digest).unwrap(),
        }
    }
}

fn hex_to_felt(hex: &str) -> Result<FieldElement, String> {
    let mut bytes = match hex::decode(hex) {
        Ok(b) => b,
        Err(e) => return Err(format!("Failed to decode hex: {}", e)),
    };

    if bytes.len() > 32 {
        return Err("Hex string too long to fit into a FieldElement".to_string());
    }
    while bytes.len() < 32 {
        bytes.insert(0, 0);
    }

    let byte_array: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "Failed to convert bytes to a 32-byte array".to_string())?;

    println!("byte_array: {:?}", byte_array);

    Ok(FieldElement::from_bytes_be(&byte_array))
}

#[test]
fn test_from_attestation_to_block() {
    let attestation = Attestation {
        chain_id: 1,
        header_number: 1,
        header_hash: "1234".to_string(),
        tx_root: "1234".to_string(),
        rx_root: "1234".to_string(),
        digest: "712407950682829515725516432181193776679273327660415695581617124654780006662"
            .to_string(),
        prev_digest: Some(
            "2294326729661400123054252499768624109855664421347212272776906071729887468097"
                .to_string(),
        ),
        signature: "1234".to_string(),
        attestors: vec![Some("1234".to_string())],
    };

    let block: Block = attestation.into();
    assert_eq!(block.block_number, 1.into());
    assert_eq!(block.tx_root, FieldElement::from_str("0x1234").unwrap());
    assert_eq!(block.rx_root, FieldElement::from_str("0x1234").unwrap());
}

pub async fn get_by_digest(
    connection: &mut AsyncPgConnection,
    digest: String,
) -> Result<Attestation> {
    signedattestation_table
        .select(Attestation::as_select())
        .filter(signedattestation::digest.eq(digest))
        .first(connection)
        .await
        .map_err(|e| {
            tracing::error!("Error getting attestation by digest: {:?}", e);
            anyhow::anyhow!(e)
        })
}

pub async fn get_by_header_number(
    connection: &mut AsyncPgConnection,
    header_number: i64,
    chain_id: i64,
) -> Result<Attestation> {
    signedattestation_table
        .select(Attestation::as_select())
        .filter(signedattestation::header_number.eq(header_number))
        .filter(signedattestation::chain_id.eq(chain_id))
        .first(connection)
        .await
        .map_err(|e| {
            tracing::error!(
                "Error getting attestation by header number: {:?} {:?}",
                header_number,
                e
            );
            anyhow::anyhow!(e)
        })
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

pub async fn first_digest_exists(
    connection: &mut AsyncPgConnection,
    chain_id: u64,
) -> Result<bool> {
    Ok(diesel::select(diesel_exists(
        signedattestation_table
            .filter(signedattestation::chain_id.eq(convert(chain_id)))
            .filter(signedattestation::prev_digest.is_null()),
    ))
    .get_result(connection)
    .await?)
}

pub async fn last_synced(
    connection: &mut AsyncPgConnection,
    chain_id: u64,
) -> Result<Option<Attestation>> {
    match signedattestation_table
        .order(signedattestation::header_number.asc())
        .filter(signedattestation::chain_id.eq(convert(chain_id)))
        .select(Attestation::as_select())
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
impl<H, A> From<SignedAttestation<H, A>> for Attestation
where
    H: AsRef<[u8]> + Clone + Copy,
    A: AsRef<[u8]> + Clone,
{
    fn from(value: SignedAttestation<H, A>) -> Self {
        Attestation {
            chain_id: convert(value.attestation.chain_id),
            header_number: convert(value.attestation.header_number),
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

#[must_use]
fn convert(num: u64) -> i64 {
    i64::from_ne_bytes(num.to_ne_bytes())
}
