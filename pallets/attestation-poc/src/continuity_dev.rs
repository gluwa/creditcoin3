use attestor_primitives::{block::Block, block::ContinuityProof, Digest};
use sp_core::H256;
use sp_runtime::traits::Zero;
use sp_std::ops::RangeInclusive;

pub fn construct_fragment(
    prev_digest: Option<Digest>,
    range: RangeInclusive<u64>,
) -> ContinuityProof {
    if range.end().is_zero() {
        return ContinuityProof::default();
    }

    let mut blocks = sp_std::vec::Vec::new();
    let mut current_prev_digest = prev_digest.unwrap_or_else(Digest::zero);

    for block_number in range {
        let block = Block::new_from_prev_digest(block_number, H256::zero(), current_prev_digest);
        log::debug!(
            "Constructed block number: {}, prev_digest: {:?}, digest: {:?}",
            block_number,
            current_prev_digest,
            block.digest()
        );
        current_prev_digest = block.digest();
        blocks.push(block);
    }

    ContinuityProof::from_blocks(blocks)
}
