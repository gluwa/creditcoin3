use sp_core::H256;

use attestor_primitives::{Attestation, ChainKey};
use eth::OrderedBlock;
use mmr::traits::MerkleTreeTrait;

// Create the attestation data from a NewBlock
#[must_use]
pub fn create(chain_key: ChainKey, new_block: &OrderedBlock) -> Attestation<H256> {
    let mt = eth::starknet_pedersen_mmr(new_block);
    Attestation {
        chain_key,
        header_number: new_block.number(),
        header_hash: sp_core::H256(*new_block.hash().unwrap()),
        root: mt.root().0.to_bytes_be(),
        // We don't have a prev_digest yet, so we set it to None
        prev_digest: None,
    }
}
