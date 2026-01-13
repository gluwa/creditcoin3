use crate::prelude::*;

use super::error::*;

pub struct AttestationBlockCache(Vec<attestor_primitives::block::BlockSerializable>);

impl AttestationBlockCache {
    pub fn new() -> Self {
        // TODO: initialize this to the max continuity proof size
        Self(Vec::new())
    }

    pub async fn fetch_blocks(
        &mut self,
        eth: &mut eth::Client,
        from_digest: attestor_primitives::Digest,
        block_start: common::types::Height,
        block_stop: common::types::Height,
    ) -> Result<Vec<attestor_primitives::block::BlockSerializable>, Error> {
        assert!(block_start <= block_stop);

        let fragment_size = (block_stop - block_start + 1) as usize;
        let mut fragment_blocks =
            Vec::<attestor_primitives::block::BlockSerializable>::with_capacity(fragment_size);

        let blocks = &mut self.0;
        if !blocks.is_empty() {
            assert_eq!(
                blocks[0].prev_digest, from_digest,
                "Invalid cached prev_digest"
            );
        }

        let copy = 0..fragment_size.min(blocks.len());
        fragment_blocks.extend_from_slice(&blocks[copy]);

        let missing_start = block_start + blocks.len() as common::types::Height;
        let missing_stop = block_stop;
        if missing_start <= missing_stop {
            Self::fetch_blocks_missing(
                eth,
                &mut fragment_blocks,
                from_digest,
                missing_start,
                missing_stop,
            )
            .await?;

            let copy = missing_start as usize..fragment_blocks.len();
            blocks.extend_from_slice(&fragment_blocks[copy]);
        }

        Ok(fragment_blocks)
    }

    async fn fetch_blocks_missing(
        eth: &mut eth::Client,
        fragment_blocks: &mut Vec<attestor_primitives::block::BlockSerializable>,
        from_digest: attestor_primitives::Digest,
        block_start: common::types::Height,
        block_stop: common::types::Height,
    ) -> Result<(), Error> {
        use futures::FutureExt as _;
        use rayon::iter::IndexedParallelIterator as _;
        use rayon::iter::IntoParallelIterator as _;
        use rayon::iter::ParallelIterator as _;

        assert!(block_start <= block_stop);

        let encoding = ccnext_abi_encoding::common::EncodingVersion::V1;
        let blocks = (block_start..=block_stop).map(|height| {
            eth.get_block(height, encoding)
                .map(|opt| opt.transpose().map_err(Error::EthClient))
        });
        let blocks = futures::future::try_join_all(blocks).await?;

        let fragment_size = (block_stop - block_start + 1) as usize;
        let mut blocks_with_roots = Vec::with_capacity(fragment_size);
        blocks
            .into_par_iter()
            .map(|opt| opt.map(|block| (eth::simple_merkle_tree(&block).root(), block)))
            .collect_into_vec(&mut blocks_with_roots);

        for opt in blocks_with_roots {
            let Some((root, block)) = opt else {
                // NOTE: INTERRUPT
                //
                // User-initiated shutdown. See the implementation of `self.eth.get_block` to
                // understand why this is here.
                todo!();
                // return None;
            };

            let block = if let Some(head) = fragment_blocks.last() {
                attestor_primitives::block::Block::new_from_prev_digest(
                    block.number(),
                    root,
                    head.digest,
                )
            } else {
                attestor_primitives::block::Block::new_from_prev_digest(
                    block.number(),
                    root,
                    from_digest,
                )
            };

            fragment_blocks.push(attestor_primitives::block::BlockSerializable::from(block));
        }

        Ok(())
    }
}

impl crate::events::EventAttestationFinalization for AttestationBlockCache {
    type Error = std::convert::Infallible;

    async fn note_attestation_finalization(
        &mut self,
        _latest_attestation_cc3: (attestor_primitives::Digest, common::types::Height),
    ) -> Result<(), Self::Error> {
        self.0.clear();
        Ok(())
    }
}
