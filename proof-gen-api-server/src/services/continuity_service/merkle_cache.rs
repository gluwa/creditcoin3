use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use merkle::keccak_merkle_tree::KeccakMerkleTree;
use sp_core::H256;
use tokio::sync::RwLock;

use super::MerkleProofItem;

/// Per-transaction cost of the cache-level tx-hash index, charged to the block that owns the
/// transaction so a block's measured size covers everything caching it allocates.
///
/// Each transaction adds one `by_tx_hash` entry: a 32-byte `H256` key plus a `(u64, usize)`
/// value, in a hashbrown table that holds spare capacity and a control byte per slot. Rounded up
/// rather than derived exactly -- the true figure moves with the table's load factor, and a
/// budget that understates its own cost is worse than one that is slightly conservative.
const PER_TX_INDEX_OVERHEAD_BYTES: usize = 64;

#[derive(Debug)]
struct CachedMerkleBlock {
    header_number: u64,
    tx_hashes: Vec<H256>,
    tx_bytes: Vec<Vec<u8>>,
    tree: KeccakMerkleTree,
    /// Approximate heap footprint, measured once at construction.
    ///
    /// Precomputed rather than derived on demand so that the running total in
    /// [`ChainMerkleCache::cached_bytes`] adds and subtracts the exact same number for a given
    /// block. Recomputing it at removal time would let the total drift.
    heap_bytes: u64,
}

impl CachedMerkleBlock {
    fn new(header_number: u64, txs: Vec<(H256, Vec<u8>)>) -> Self {
        let (tx_hashes, tx_bytes): (Vec<_>, Vec<_>) = txs.into_iter().unzip();
        let tree = KeccakMerkleTree::new(&tx_bytes);
        let heap_bytes = Self::measure_heap_bytes(&tx_hashes, &tx_bytes, &tree);

        Self {
            header_number,
            tx_hashes,
            tx_bytes,
            tree,
            heap_bytes,
        }
    }

    /// Approximate heap bytes held by one cached block.
    ///
    /// The dominant term is the raw transaction payloads; the hash vector and the merkle tree
    /// are both `O(tx_count)` in 32-byte words. Exact allocator overhead is not modelled -- this
    /// feeds a budget, so a consistent underestimate is fine as long as it tracks tx density.
    fn measure_heap_bytes(
        tx_hashes: &[H256],
        tx_bytes: &[Vec<u8>],
        tree: &KeccakMerkleTree,
    ) -> u64 {
        let hashes = std::mem::size_of::<H256>().saturating_mul(tx_hashes.len());
        let payloads = tx_bytes.iter().fold(0usize, |acc, tx| {
            acc.saturating_add(tx.len())
                .saturating_add(std::mem::size_of::<Vec<u8>>())
        });
        let index = PER_TX_INDEX_OVERHEAD_BYTES.saturating_mul(tx_hashes.len());

        hashes
            .saturating_add(payloads)
            .saturating_add(index)
            .saturating_add(tree.heap_bytes()) as u64
    }

    async fn build(header_number: u64, txs: Vec<(H256, Vec<u8>)>) -> Result<Self, String> {
        tokio::task::spawn_blocking(move || Self::new(header_number, txs))
            .await
            .map_err(|err| format!("merkle cache build task panicked: {err}"))
    }

    fn proof_item(&self, chain_key: u64, tx_index: usize) -> Option<MerkleProofItem> {
        let tx_hash = *self.tx_hashes.get(tx_index)?;
        let tx_bytes = self.tx_bytes.get(tx_index)?.clone();
        let merkle_proof = self.tree.generate_proof(tx_index).ok()?;

        Some(MerkleProofItem {
            chain_key,
            header_number: self.header_number,
            tx_index: Some(tx_index as u64),
            tx_hash: Some(tx_hash),
            tx_bytes: Some(tx_bytes),
            merkle_root: merkle_proof.root,
            merkle_proof,
        })
    }
}

#[derive(Debug, Default)]
struct ChainMerkleCache {
    by_block: BTreeMap<u64, Arc<CachedMerkleBlock>>,
    by_tx_hash: HashMap<H256, (u64, usize)>,
    processed_blocks: BTreeSet<u64>,
    /// Running sum of [`CachedMerkleBlock::heap_bytes`] over `by_block`.
    ///
    /// Maintained incrementally because the whole point is to read it cheaply on every backfill
    /// tick; summing the map each time would be `O(blocks)` on the hot path.
    cached_bytes: u64,
}

impl ChainMerkleCache {
    /// Drop a cached block and everything indexing it, keeping `cached_bytes` in step.
    ///
    /// Note this deliberately leaves `processed_blocks` alone: a height can be "processed" with
    /// no cached block (an empty source block), and the backfill worker uses that set to decide
    /// what still needs fetching.
    fn drop_block(&mut self, header_number: u64) -> bool {
        let Some(old) = self.by_block.remove(&header_number) else {
            return false;
        };

        self.cached_bytes = self.cached_bytes.saturating_sub(old.heap_bytes);
        for tx_hash in &old.tx_hashes {
            self.by_tx_hash.remove(tx_hash);
        }

        true
    }
}

/// Snapshot of a chain's merkle cache occupancy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MerkleCacheStats {
    pub blocks: u64,
    pub txs: u64,
    pub bytes: u64,
}

impl MerkleCacheStats {
    /// Mean heap bytes per cached block, or `None` while the cache is still cold.
    ///
    /// This is the measured density that lets a byte budget be translated into a block count
    /// without assuming anything about the chain.
    pub fn mean_bytes_per_block(&self) -> Option<u64> {
        if self.blocks == 0 || self.bytes == 0 {
            return None;
        }

        Some((self.bytes / self.blocks).max(1))
    }
}

/// In-memory cache of finalized source-chain merkle data.
///
/// Stores one reusable merkle tree per processed block and a tx-hash index into
/// those blocks. This avoids storing every per-transaction proof while making a
/// tx-hash cache hit cheap to serve.
#[derive(Debug, Default)]
pub struct MerkleProofCache {
    inner: RwLock<ChainMerkleCache>,
}

impl MerkleProofCache {
    pub async fn get_by_tx_hash(&self, chain_key: u64, tx_hash: H256) -> Option<MerkleProofItem> {
        let cache = self.inner.read().await;
        let (header_number, tx_index) = *cache.by_tx_hash.get(&tx_hash)?;
        let block = cache.by_block.get(&header_number)?;
        block.proof_item(chain_key, tx_index)
    }

    pub async fn get_by_block_index(
        &self,
        chain_key: u64,
        header_number: u64,
        tx_index: u64,
    ) -> Option<MerkleProofItem> {
        let cache = self.inner.read().await;
        let block = cache.by_block.get(&header_number)?;
        block.proof_item(chain_key, tx_index as usize)
    }

    pub async fn insert_block(
        &self,
        header_number: u64,
        txs: Vec<(H256, Vec<u8>)>,
    ) -> Result<usize, String> {
        let block = Arc::new(CachedMerkleBlock::build(header_number, txs).await?);
        let tx_count = block.tx_hashes.len();

        self.insert_cached_block(header_number, block).await;

        Ok(tx_count)
    }

    pub async fn insert_block_and_get(
        &self,
        chain_key: u64,
        header_number: u64,
        txs: Vec<(H256, Vec<u8>)>,
        tx_index: u64,
    ) -> Result<(usize, Option<MerkleProofItem>), String> {
        let block = Arc::new(CachedMerkleBlock::build(header_number, txs).await?);
        let tx_count = block.tx_hashes.len();
        let item = block.proof_item(chain_key, tx_index as usize);

        self.insert_cached_block(header_number, block).await;

        Ok((tx_count, item))
    }

    async fn insert_cached_block(&self, header_number: u64, block: Arc<CachedMerkleBlock>) {
        let mut cache = self.inner.write().await;
        // Re-inserting a height already cached is normal (an on-demand fill can race a
        // backfill), so the old block's bytes must come off before the new block's go on.
        cache.drop_block(header_number);

        for (tx_index, tx_hash) in block.tx_hashes.iter().copied().enumerate() {
            cache.by_tx_hash.insert(tx_hash, (header_number, tx_index));
        }
        cache.processed_blocks.insert(header_number);
        cache.cached_bytes = cache.cached_bytes.saturating_add(block.heap_bytes);
        cache.by_block.insert(header_number, block);
    }

    pub async fn mark_processed_empty(&self, header_number: u64) {
        let mut cache = self.inner.write().await;
        cache.drop_block(header_number);
        cache.processed_blocks.insert(header_number);
    }

    /// Current occupancy of this chain's cache.
    pub async fn size_stats(&self) -> MerkleCacheStats {
        let cache = self.inner.read().await;

        MerkleCacheStats {
            blocks: cache.by_block.len() as u64,
            txs: cache.by_tx_hash.len() as u64,
            bytes: cache.cached_bytes,
        }
    }

    pub async fn is_processed(&self, header_number: u64) -> bool {
        self.inner
            .read()
            .await
            .processed_blocks
            .contains(&header_number)
    }

    pub async fn next_unprocessed_height(&self, start: u64, end: u64) -> Option<u64> {
        let cache = self.inner.read().await;
        (start..=end).find(|height| !cache.processed_blocks.contains(height))
    }

    pub async fn next_unprocessed_height_desc(&self, start: u64, end: u64) -> Option<u64> {
        let cache = self.inner.read().await;
        (start..=end)
            .rev()
            .find(|height| !cache.processed_blocks.contains(height))
    }

    pub async fn unprocessed_heights_desc(&self, start: u64, end: u64, limit: usize) -> Vec<u64> {
        let cache = self.inner.read().await;
        (start..=end)
            .rev()
            .filter(|height| !cache.processed_blocks.contains(height))
            .take(limit)
            .collect()
    }

    pub async fn prune_below(&self, min_height: u64) -> usize {
        let mut cache = self.inner.write().await;
        let kept_blocks = cache.by_block.split_off(&min_height);
        let removed_blocks = std::mem::replace(&mut cache.by_block, kept_blocks);

        let kept_processed = cache.processed_blocks.split_off(&min_height);
        cache.processed_blocks = kept_processed;

        let removed = removed_blocks.len();
        for block in removed_blocks.into_values() {
            cache.cached_bytes = cache.cached_bytes.saturating_sub(block.heap_bytes);
            for tx_hash in &block.tx_hashes {
                cache.by_tx_hash.remove(tx_hash);
            }
        }

        removed
    }

    pub async fn prune_above(&self, max_height: u64) -> usize {
        let split_key = max_height.saturating_add(1);
        let mut cache = self.inner.write().await;
        let removed_blocks = cache.by_block.split_off(&split_key);
        cache.processed_blocks.split_off(&split_key);

        let removed = removed_blocks.len();
        for block in removed_blocks.into_values() {
            cache.cached_bytes = cache.cached_bytes.saturating_sub(block.heap_bytes);
            for tx_hash in &block.tx_hashes {
                cache.by_tx_hash.remove(tx_hash);
            }
        }

        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tx(n: u64) -> (H256, Vec<u8>) {
        (H256::from_low_u64_be(n), vec![n as u8])
    }

    #[tokio::test]
    async fn lookup_returns_proof_for_cached_tx() {
        let cache = MerkleProofCache::default();
        cache
            .insert_block(100, vec![tx(1), tx(2), tx(3)])
            .await
            .unwrap();

        let item = cache
            .get_by_tx_hash(7, H256::from_low_u64_be(2))
            .await
            .expect("tx should be cached");

        assert_eq!(item.chain_key, 7);
        assert_eq!(item.header_number, 100);
        assert_eq!(item.tx_index, Some(1));
        assert_eq!(item.tx_hash, Some(H256::from_low_u64_be(2)));
        assert_eq!(item.tx_bytes, Some(vec![2]));
        assert!(item
            .merkle_proof
            .verify(item.tx_bytes.as_deref().expect("tx bytes cached")));
    }

    #[tokio::test]
    async fn lookup_by_block_index_returns_proof_for_cached_tx() {
        let cache = MerkleProofCache::default();
        cache
            .insert_block(100, vec![tx(1), tx(2), tx(3)])
            .await
            .unwrap();

        let item = cache
            .get_by_block_index(7, 100, 2)
            .await
            .expect("tx index should be cached");

        assert_eq!(item.chain_key, 7);
        assert_eq!(item.header_number, 100);
        assert_eq!(item.tx_index, Some(2));
        assert_eq!(item.tx_hash, Some(H256::from_low_u64_be(3)));
        assert_eq!(item.tx_bytes, Some(vec![3]));
    }

    #[tokio::test]
    async fn prune_below_removes_tx_index_and_processed_blocks() {
        let cache = MerkleProofCache::default();
        cache.insert_block(100, vec![tx(1)]).await.unwrap();
        cache.insert_block(200, vec![tx(2)]).await.unwrap();

        assert_eq!(cache.prune_below(150).await, 1);

        assert!(cache
            .get_by_tx_hash(7, H256::from_low_u64_be(1))
            .await
            .is_none());
        assert!(cache
            .get_by_tx_hash(7, H256::from_low_u64_be(2))
            .await
            .is_some());
        assert!(!cache.is_processed(100).await);
        assert!(cache.is_processed(200).await);
    }

    #[tokio::test]
    async fn prune_above_removes_reverted_blocks() {
        let cache = MerkleProofCache::default();
        cache.insert_block(100, vec![tx(1)]).await.unwrap();
        cache.insert_block(200, vec![tx(2)]).await.unwrap();

        assert_eq!(cache.prune_above(150).await, 1);

        assert!(cache
            .get_by_tx_hash(7, H256::from_low_u64_be(1))
            .await
            .is_some());
        assert!(cache
            .get_by_tx_hash(7, H256::from_low_u64_be(2))
            .await
            .is_none());
        assert!(cache.is_processed(100).await);
        assert!(!cache.is_processed(200).await);
    }

    #[tokio::test]
    async fn mark_processed_empty_removes_stale_cached_block() {
        let cache = MerkleProofCache::default();
        cache.insert_block(100, vec![tx(1), tx(2)]).await.unwrap();

        cache.mark_processed_empty(100).await;

        assert!(cache
            .get_by_tx_hash(7, H256::from_low_u64_be(1))
            .await
            .is_none());
        assert!(cache.get_by_block_index(7, 100, 0).await.is_none());
        assert!(cache.is_processed(100).await);
    }

    #[tokio::test]
    async fn next_unprocessed_height_desc_prefers_newest_missing_block() {
        let cache = MerkleProofCache::default();
        cache.mark_processed_empty(10).await;
        cache.mark_processed_empty(12).await;

        assert_eq!(cache.next_unprocessed_height_desc(10, 12).await, Some(11));

        cache.mark_processed_empty(11).await;
        assert_eq!(cache.next_unprocessed_height_desc(10, 12).await, None);
    }

    #[tokio::test]
    async fn unprocessed_heights_desc_returns_newest_missing_blocks() {
        let cache = MerkleProofCache::default();
        cache.mark_processed_empty(11).await;
        cache.mark_processed_empty(14).await;

        assert_eq!(
            cache.unprocessed_heights_desc(10, 14, 3).await,
            vec![13, 12, 10]
        );
    }

    /// Variable-size payload: the default `tx` helper makes 1-byte payloads, which are too
    /// small to tell a real byte total from a per-entry constant.
    fn sized_tx(n: u64, len: usize) -> (H256, Vec<u8>) {
        (H256::from_low_u64_be(n), vec![n as u8; len])
    }

    #[tokio::test]
    async fn byte_accounting_returns_to_zero_after_pruning_everything() {
        let cache = MerkleProofCache::default();
        cache
            .insert_block(10, vec![sized_tx(1, 512), sized_tx(2, 512)])
            .await
            .expect("insert should succeed");
        cache
            .insert_block(11, vec![sized_tx(3, 512)])
            .await
            .expect("insert should succeed");

        let filled = cache.size_stats().await;
        assert_eq!(filled.blocks, 2);
        assert_eq!(filled.txs, 3);
        assert!(
            filled.bytes >= 3 * 512,
            "payloads should dominate: {filled:?}"
        );

        cache.prune_below(100).await;

        assert_eq!(cache.size_stats().await, MerkleCacheStats::default());
    }

    #[tokio::test]
    async fn reinserting_same_height_does_not_drift_byte_total() {
        // An on-demand fill can re-cache a height a backfill already cached. If the old block's
        // bytes were not subtracted first, the total would grow on every re-insert.
        let cache = MerkleProofCache::default();
        cache
            .insert_block(10, vec![sized_tx(1, 1024)])
            .await
            .expect("insert should succeed");
        let first = cache.size_stats().await;

        for _ in 0..5 {
            cache
                .insert_block(10, vec![sized_tx(1, 1024)])
                .await
                .expect("insert should succeed");
        }

        assert_eq!(cache.size_stats().await, first);
    }

    #[tokio::test]
    async fn marking_a_cached_height_empty_releases_its_bytes() {
        let cache = MerkleProofCache::default();
        cache
            .insert_block(10, vec![sized_tx(1, 1024)])
            .await
            .expect("insert should succeed");
        assert!(cache.size_stats().await.bytes > 0);

        cache.mark_processed_empty(10).await;

        let stats = cache.size_stats().await;
        assert_eq!(stats.bytes, 0);
        assert_eq!(stats.blocks, 0);
        assert_eq!(stats.txs, 0);
        // Still processed: an empty block needs no refetch.
        assert!(cache.is_processed(10).await);
    }

    #[tokio::test]
    async fn prune_above_releases_bytes_for_reverted_blocks() {
        let cache = MerkleProofCache::default();
        for height in 10..=12 {
            cache
                .insert_block(height, vec![sized_tx(height, 1024)])
                .await
                .expect("insert should succeed");
        }
        let all_three = cache.size_stats().await.bytes;

        cache.prune_above(10).await;

        let stats = cache.size_stats().await;
        assert_eq!(stats.blocks, 1);
        assert!(
            stats.bytes > 0 && stats.bytes < all_three,
            "one of three blocks should remain: {stats:?} vs {all_three}"
        );

        cache.prune_above(9).await;
        assert_eq!(cache.size_stats().await, MerkleCacheStats::default());
    }

    #[tokio::test]
    async fn byte_total_tracks_transaction_density() {
        // The whole point of the budget: two chains with the same block count but different tx
        // density must report very different totals.
        let sparse = MerkleProofCache::default();
        sparse
            .insert_block(1, vec![sized_tx(1, 256)])
            .await
            .expect("insert should succeed");

        let dense = MerkleProofCache::default();
        dense
            .insert_block(1, (0..100).map(|n| sized_tx(n, 256)).collect())
            .await
            .expect("insert should succeed");

        let sparse_stats = sparse.size_stats().await;
        let dense_stats = dense.size_stats().await;

        assert_eq!(sparse_stats.blocks, dense_stats.blocks);
        assert!(
            dense_stats.bytes > sparse_stats.bytes * 50,
            "dense={dense_stats:?} sparse={sparse_stats:?}"
        );
    }

    #[tokio::test]
    async fn mean_bytes_per_block_is_none_while_cold() {
        let cache = MerkleProofCache::default();
        assert_eq!(cache.size_stats().await.mean_bytes_per_block(), None);

        cache
            .insert_block(1, vec![sized_tx(1, 1024)])
            .await
            .expect("insert should succeed");

        let mean = cache
            .size_stats()
            .await
            .mean_bytes_per_block()
            .expect("warm cache should report a mean");
        assert!(mean >= 1024, "mean should cover the payload: {mean}");
    }
}
