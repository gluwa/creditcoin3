use anyhow::Result;
use attestation_chain::block::Block;
use starknet_types_core::felt::Felt as FieldElement;
use tracing::debug;
use utils::Felt;

use crate::postgres;

#[allow(dead_code)]
pub enum ConversionError {
    InvalidTxRoot,
    InvalidRxRoot,
    InvalidPrevDigest,
    InvalidDigest,
}

pub fn create_block_with_prev_digest(
    attestation: &postgres::blockwithdigest::BlockWithDigest,
    prev_digest: &str,
) -> Result<Block> {
    let root = Felt::from_hex(&attestation.merkle_root)?;
    debug!("created merkle_root: {:?}", root);

    let prev_digest = Felt::from_hex(prev_digest)?;
    debug!("created prev_digest: {:?}", prev_digest);

    let digest = Felt::from_hex(&format!("0x{0}", attestation.digest))?;
    debug!("digest: {:?}", digest);

    Ok(Block {
        block_number: attestation.header_number as u64,
        root,
        prev_digest,
        digest,
    })
}

impl From<crate::postgres::attestation::Attestation> for Block {
    fn from(attestation: crate::postgres::attestation::Attestation) -> Self {
        debug!("Converting attestation to block: {:?}", attestation);
        debug!("merkle_root : {:?}", attestation.merkle_root);
        debug!("tx_root str: {:?}", attestation.merkle_root.as_str());

        Block {
            block_number: attestation.header_number as u64,
            root: hex_to_felt(&attestation.merkle_root).unwrap(),
            prev_digest: FieldElement::from_dec_str(&attestation.prev_digest.expect("Some"))
                .unwrap(),
            digest: FieldElement::from_dec_str(&attestation.digest).unwrap(),
        }
    }
}

fn hex_to_felt(hex: &str) -> anyhow::Result<FieldElement, String> {
    let mut bytes = match hex::decode(hex) {
        Ok(b) => b,
        Err(e) => return Err(format!("Failed to decode hex: {e}")),
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

    Ok(FieldElement::from_bytes_be(&byte_array))
}

#[test]
fn test_from_attestation_to_block() {
    let attestation = crate::postgres::attestation::Attestation {
        chain_key: 1,
        header_number: 1,
        header_hash: "1234".to_string(),
        merkle_root: "1234".to_string(),
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
    assert_eq!(block.block_number, 1u64);
    assert_eq!(block.root, FieldElement::from_hex("0x1234").unwrap());
}
