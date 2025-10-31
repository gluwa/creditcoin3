use attestor_primitives::{attestation_fragment::AttestationFragment, block::Block, Digest};
use sp_core::H256;
use sp_runtime::traits::Zero;
use sp_std::ops::RangeInclusive;

pub fn construct_fragment(
    prev_digest: Option<Digest>,
    range: RangeInclusive<u64>,
) -> AttestationFragment {
    if range.end().is_zero() {
        return AttestationFragment::default();
    }

    // Create a dummy fragment from start to end and use provided digest if we can
    let mut fragment = AttestationFragment::new(range.clone().count());
    let mut current_prev_digest = prev_digest.unwrap_or_else(Digest::zero);

    for block_number in range {
        let block = Block::new_from_prev_digest(block_number, H256::zero(), current_prev_digest);
        log::debug!(
            "Constructed block number: {}, prev_digest: {:?}, digest: {:?}",
            block_number,
            current_prev_digest,
            block.digest()
        );
        let appended_block = fragment
            .try_append_block(block)
            .expect("Failed to append block");
        current_prev_digest = appended_block.digest();
    }

    fragment
}
