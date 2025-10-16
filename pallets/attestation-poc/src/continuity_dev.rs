use attestor_primitives::{attestation_fragment::AttestationFragment, block::Block, Digest};
use sp_runtime::traits::Zero;
use sp_std::ops::RangeInclusive;
use starknet_crypto::Felt;

pub fn construct_fragment(
    prev_digest: Option<Digest>,
    range: RangeInclusive<u64>,
) -> AttestationFragment {
    if range.end().is_zero() {
        return AttestationFragment::default();
    }
    // Create a dummy fragment from start to end and use provided digest if we can
    let mut fragment = AttestationFragment::new((range.end() - range.start() + 1) as usize);
    let mut current_prev_digest =
        Felt::from_bytes_be(&prev_digest.map(|d| d.0).unwrap_or_default());

    for block_number in range {
        let block = Block::new_from_prev_digest(block_number, Felt::default(), current_prev_digest);
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
