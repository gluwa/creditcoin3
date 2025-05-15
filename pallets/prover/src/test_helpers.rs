use attestor_primitives::{Attestation as AttestationPrimitive, ChainKey, SignedAttestation};
use sp_core::H256;
use sp_std::vec::Vec;

pub const PROOF_EXAMPLE_DIGEST_HEX: &str =
    "0x0356f5b61833ff38a0322cfea30f745d0948a7b1baaff455b7ea34e41812f560";

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
