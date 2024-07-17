use attestation_chain::block::Block;
use parity_scale_codec::{Decode, Encode};
use sp_core::H256;
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
