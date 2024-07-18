use attestation_chain::block::Block;
use parity_scale_codec::{Decode, Encode};
use sp_core::H256;
use starknet_types_core::felt::Felt as FieldElement;
use tracing::debug;
use utils::Felt;

use attestor_primitives::SignedAttestation;

pub struct Attestation<H, A>(SignedAttestation<H, A>);

pub enum ConversionError {
    InvalidTxRoot,
    InvalidRxRoot,
    InvalidPrevDigest,
    InvalidDigest,
}

// Implement into Block for SignedAttestation
// To facilitate conversion from runtime attestation to block type which is used by the prover library
impl<H, A> TryInto<Block> for Attestation<H, A>
where
    H: Into<H256> + AsRef<[u8]>,
    A: Encode + Decode,
{
    type Error = ConversionError;

    fn try_into(self) -> Result<Block, Self::Error> {
        let attestation = self.0;

        let prev_digest = if let Some(prev_digest) = attestation.attestation.prev_digest {
            prev_digest
        } else {
            H256::default()
        };

        let digest = attestation.digest();

        let tx_root = Felt::from_bytes_be(&attestation.attestation.tx_root);

        let rx_root = Felt::from_bytes_be(&attestation.attestation.rx_root);

        let prev_digest = Felt::from_bytes_be(&prev_digest.0);

        let digest = Felt::from_bytes_be(&digest.0);

        Ok(Block {
            block_number: attestation.attestation.header_number.into(),
            tx_root,
            rx_root,
            prev_digest,
            digest,
        })
    }
}

impl From<crate::postgres::attestation::Attestation> for Block {
    fn from(attestation: crate::postgres::attestation::Attestation) -> Self {
        debug!("Converting attestation to block: {:?}", attestation);
        debug!("tx_root : {:?}", attestation.tx_root);
        debug!("tx_root str: {:?}", attestation.tx_root.as_str());

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

fn hex_to_felt(hex: &str) -> anyhow::Result<FieldElement, String> {
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

    Ok(FieldElement::from_bytes_be(&byte_array))
}

#[test]
fn test_from_attestation_to_block() {
    let attestation = crate::postgres::attestation::Attestation {
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
