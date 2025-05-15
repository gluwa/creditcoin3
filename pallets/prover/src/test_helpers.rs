use attestor_primitives::{Attestation as AttestationPrimitive, ChainKey, SignedAttestation};
use sp_core::H256;
use sp_std::vec::Vec;

pub const PROOF_EXAMPLE_DIGEST_HEX: &str =
    "1961425310546718893260438760555883585418372007745579061792800243857663376404";

pub fn create_dummy_attestation<AccountId>(
    chain_key: ChainKey,
    header_number: u64,
    prev_digest: Option<H256>,
) -> SignedAttestation<H256, AccountId> {
    let attestation = AttestationPrimitive {
        chain_key,
        header_number,
        header_hash: H256::zero(),
        root: [0; 32],
        prev_digest,
    };

    SignedAttestation {
        attestation,
        signature: [0u8; 96],
        attestors: Vec::new(),
    }
}
