use sp_core::H256;

use attestor_primitives::{Attestation, ChainKey};
use eth::OrderedBlock;

use tracing::debug;

pub mod retry;
pub mod sanitize_url;

// Create the attestation data from a NewBlock
#[must_use]
pub fn create_attestation(
    chain_key: ChainKey,
    new_block: &OrderedBlock,
    prev_digest: Option<H256>,
) -> Attestation<H256> {
    let tree = eth::simple_merkle_tree(new_block);
    let root = tree.root();

    debug!(
        "Header hash: {:?}",
        sp_core::H256(*new_block.hash().unwrap())
    );
    Attestation {
        chain_key,
        header_number: new_block.number(),
        header_hash: sp_core::H256(*new_block.hash().unwrap()),
        root,
        prev_digest,
    }
}
