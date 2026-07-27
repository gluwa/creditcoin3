use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use merkle::keccak_merkle_tree::KeccakMerkleTree;
use sp_core::H256;
use tokio::sync::RwLock;

use super::MerkleProofItem;

#[derive(Debug)]
struct CachedMerkleBlock {
    header_number: u64,
    tx_hashes: Vec<H256>,
    tx_bytes: Vec<Vec<u8>>,
    tree: KeccakMerkleTree,
}

impl CachedMerkleBlock {
    fn new(header_number: u64, txs: Vec<(H256, Vec<u8>)>) -> Self {
        let (tx_hashes, tx_bytes): (Vec<_>, Vec<_>) = txs.into_iter().unzip();
        let tree = KeccakMerkleTree::new(&tx_bytes);

        Self {
            header_number,
            tx_hashes,
            tx_bytes,
            tree,
        }
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
        if let Some(old) = cache.by_block.remove(&header_number) {
            for tx_hash in &old.tx_hashes {
                cache.by_tx_hash.remove(tx_hash);
            }
        }

        for (tx_index, tx_hash) in block.tx_hashes.iter().copied().enumerate() {
            cache.by_tx_hash.insert(tx_hash, (header_number, tx_index));
        }
        cache.processed_blocks.insert(header_number);
        cache.by_block.insert(header_number, block);
    }

    pub async fn mark_processed_empty(&self, header_number: u64) {
        let mut cache = self.inner.write().await;
        if let Some(old) = cache.by_block.remove(&header_number) {
            for tx_hash in &old.tx_hashes {
                cache.by_tx_hash.remove(tx_hash);
            }
        }
        cache.processed_blocks.insert(header_number);
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
        if removed == 0 {
            return 0;
        }

        for block in removed_blocks.into_values() {
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
}
