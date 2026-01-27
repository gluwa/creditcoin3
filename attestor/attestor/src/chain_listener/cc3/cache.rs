use crate::prelude::*;

use super::error::*;

pub struct AttestationBlockCache {
    blocks: Vec<attestor_primitives::block::BlockSerializable>,
    max_size: usize,

    attestation_local: Option<(attestor_primitives::Digest, common::types::Height)>,
    attestation_interval: std::num::NonZero<common::types::Height>,
    checkpoint_interval: std::num::NonZero<common::types::Height>,
}

impl AttestationBlockCache {
    pub fn new(
        attestation_interval: std::num::NonZero<common::types::Height>,
        checkpoint_interval: std::num::NonZero<common::types::Height>,
    ) -> Self {
        // let max_size = attestation_interval.get() as usize * checkpoint_interval.get() as usize;
        let max_size = 10;
        Self {
            blocks: Vec::with_capacity(max_size),
            max_size,

            attestation_local: None,
            attestation_interval,
            checkpoint_interval,
        }
    }

    pub async fn fetch_blocks(
        &mut self,
        eth: &mut eth::Client,
        from_digest: attestor_primitives::Digest,
        block_start: common::types::Height,
        block_stop: common::types::Height,
    ) -> Result<Vec<attestor_primitives::block::BlockSerializable>, Error> {
        let fragment_size = block_stop.saturating_sub(block_start).saturating_add(1) as usize;
        let (fragment_size, from_digest, block_start) = {
            if fragment_size > self.max_size {
                let (from_digest, from_block) = self
                    .attestation_local
                    .expect("Genesis attestation is guaranteed to have finalized by this point");

                let block_start = from_block.saturating_add(1);
                let fragment_size =
                    block_stop.saturating_sub(block_start).saturating_add(1) as usize;

                (fragment_size, from_digest, block_start)
            } else {
                (fragment_size, from_digest, block_start)
            }
        };

        assert!(block_start <= block_stop, "{block_start} <= {block_stop}");

        let mut fragment_blocks =
            Vec::<attestor_primitives::block::BlockSerializable>::with_capacity(fragment_size);

        let blocks = &mut self.blocks;
        if !blocks.is_empty() {
            let block_first = blocks[0].block_number;
            assert!(block_first <= block_start, "{block_first} <= {block_start}");

            let delta = blocks.len().min((block_start - block_first) as usize);
            blocks.drain(..delta);

            let segment = 0..fragment_size.min(blocks.len());
            if !segment.is_empty() {
                tracing::info!(
                    start = segment.start,
                    stop = segment.end,
                    "🎯 Continuity cache hit"
                );
            }

            fragment_blocks.extend_from_slice(&blocks[segment]);
        }

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

            let segment = blocks.len()..fragment_blocks.len();
            blocks.extend_from_slice(&fragment_blocks[segment]);
        }

        let len = blocks.len();
        let max_size = self.max_size;
        assert!(len < max_size, "{len} < {max_size}");

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

        let fragment_size = (block_stop - block_start).saturating_add(1) as usize;
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

impl crate::events::EventAttestationProductionAsync for AttestationBlockCache {
    type Error = std::convert::Infallible;

    async fn note_attestation_production_async(
        &mut self,
        attestation_latest_eth: (attestor_primitives::Digest, common::types::Height),
    ) -> Result<(), Self::Error> {
        tracing::debug!("Updating continuity cache");
        self.attestation_local = Some(attestation_latest_eth);
        Ok(())
    }
}
impl crate::events::EventAttestationProduction for AttestationBlockCache {}

impl crate::events::EventAttestationFinalizationAsync for AttestationBlockCache {
    type Error = std::convert::Infallible;

    async fn note_attestation_finalization_async(
        &mut self,
        _attestation_latest_cc3: (attestor_primitives::Digest, common::types::Height),
    ) -> Result<(), Self::Error> {
        tracing::debug!("Updating continuity cache");
        self.blocks.clear();
        Ok(())
    }
}
impl crate::events::EventAttestationFinalization for AttestationBlockCache {}

impl crate::events::EventAttestationIntervalChangeAsync for AttestationBlockCache {
    type Error = std::convert::Infallible;

    async fn note_attestation_interval_change_async(
        &mut self,
        interval_new: std::num::NonZero<common::types::Height>,
        _attestation_latest_cc3: Option<common::types::Height>,
    ) -> Result<(), Self::Error> {
        tracing::debug!("Updating continuity cache");

        self.attestation_interval = interval_new;
        self.max_size = interval_new.get() as usize * self.checkpoint_interval.get() as usize;

        Ok(())
    }
}
impl crate::events::EventAttestationIntervalChange for AttestationBlockCache {}

impl crate::events::EventCheckpointIntervalChangeAsync for AttestationBlockCache {
    type Error = std::convert::Infallible;

    async fn note_checkpoint_interval_change_async(
        &mut self,
        interval_new: std::num::NonZero<common::types::Height>,
        _attestation_latest_cc3: Option<common::types::Height>,
    ) -> Result<(), Self::Error> {
        tracing::debug!("Updating continuity cache");

        self.checkpoint_interval = interval_new;
        self.max_size = self.attestation_interval.get() as usize * interval_new.get() as usize;

        Ok(())
    }
}
impl crate::events::EventCheckpointIntervalChange for AttestationBlockCache {}
