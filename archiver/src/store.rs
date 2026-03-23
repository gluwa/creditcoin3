//! Sled-backed root store.
//!
//! Schema: key = block height (u64 big-endian, 8 bytes), value = merkle root (H256, 32 bytes).
//! Big-endian keys ensure sled's sorted iteration yields blocks in height order.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use sp_core::H256;

/// Thread-safe handle to the root store. Cheap to clone (wraps Arc<sled::Db>).
#[derive(Clone)]
pub struct RootStore {
    db: Arc<sled::Db>,
    /// Cached entry count — avoids O(n) scan on every status request.
    entry_count: Arc<AtomicUsize>,
}

impl RootStore {
    /// Open (or create) the sled database at the given path.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let db = sled::open(path.as_ref())
            .with_context(|| format!("failed to open sled database at {:?}", path.as_ref()))?;
        // Pay the O(n) cost once at startup to seed the cached count.
        let initial_count = db.len();
        Ok(Self {
            db: Arc::new(db),
            entry_count: Arc::new(AtomicUsize::new(initial_count)),
        })
    }

    /// Insert a batch of roots atomically.
    pub fn put_roots(&self, roots: &[(u64, H256)]) -> Result<()> {
        let mut batch = sled::Batch::default();
        for (height, root) in roots {
            batch.insert(&height.to_be_bytes(), root.as_bytes());
        }
        self.db
            .apply_batch(batch)
            .context("failed to apply batch insert")?;
        // Assumes inserts are unique (no overwrites). For backfill over existing
        // entries this may drift slightly, but that's acceptable for a status counter.
        self.entry_count.fetch_add(roots.len(), Ordering::AcqRel);
        Ok(())
    }

    /// Get roots for an inclusive block range [from, to].
    /// Returns (block_number, merkle_root) pairs in ascending order.
    pub fn get_range(&self, from: u64, to: u64) -> Result<Vec<(u64, H256)>> {
        let start = from.to_be_bytes();
        let capacity = (to.saturating_sub(from) + 1) as usize;
        let mut results = Vec::with_capacity(capacity);

        for item in self.db.range(start..=to.to_be_bytes()) {
            let (key, value) = item.context("failed to read from sled")?;
            let height = parse_height(&key)?;
            let root = parse_h256(&value)?;
            results.push((height, root));
        }

        Ok(results)
    }

    /// Get the latest (highest) stored block height, or None if empty.
    pub fn latest_height(&self) -> Result<Option<u64>> {
        match self.db.last()? {
            Some((key, _)) => Ok(Some(parse_height(&key)?)),
            None => Ok(None),
        }
    }

    /// Find gaps in the stored block range.
    /// Returns a list of `(start, end)` inclusive ranges that are missing.
    pub fn find_gaps(&self) -> Result<Vec<(u64, u64)>> {
        let mut gaps = Vec::new();
        let mut expected: Option<u64> = None;

        for item in self.db.iter() {
            let (key, _) = item.context("failed to read from sled")?;
            let height = parse_height(&key)?;

            match expected {
                Some(exp) if height > exp => gaps.push((exp, height - 1)),
                _ => {}
            }
            expected = Some(height + 1);
        }

        Ok(gaps)
    }

    /// Return the cached entry count (O(1), updated on each `put_roots` call).
    pub fn count(&self) -> usize {
        self.entry_count.load(Ordering::Acquire)
    }

    /// Flush database to disk.
    pub async fn flush(&self) -> Result<()> {
        self.db.flush_async().await?;
        Ok(())
    }
}

fn parse_height(key: &sled::IVec) -> Result<u64> {
    let bytes: [u8; 8] = key
        .as_ref()
        .try_into()
        .with_context(|| format!("invalid key length: expected 8, got {}", key.len()))?;
    Ok(u64::from_be_bytes(bytes))
}

fn parse_h256(value: &sled::IVec) -> Result<H256> {
    anyhow::ensure!(
        value.len() == 32,
        "invalid digest length: expected 32, got {}",
        value.len()
    );
    Ok(H256::from_slice(value.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_put_get() {
        let dir = tempfile::tempdir().unwrap();
        let store = RootStore::open(dir.path().join("test.sled")).unwrap();

        let root = H256::random();
        store.put_roots(&[(42, root)]).unwrap();

        let range = store.get_range(42, 42).unwrap();
        assert_eq!(range.len(), 1);
        assert_eq!(range[0], (42, root));
        assert_eq!(store.get_range(43, 43).unwrap().len(), 0);
        assert_eq!(store.latest_height().unwrap(), Some(42));
    }

    #[test]
    fn range_query() {
        let dir = tempfile::tempdir().unwrap();
        let store = RootStore::open(dir.path().join("test.sled")).unwrap();

        let roots: Vec<H256> = (0..10).map(|_| H256::random()).collect();
        let entries: Vec<(u64, H256)> = roots
            .iter()
            .enumerate()
            .map(|(i, r)| (i as u64, *r))
            .collect();
        store.put_roots(&entries).unwrap();

        let range = store.get_range(3, 6).unwrap();
        assert_eq!(range.len(), 4);
        assert_eq!(range[0], (3, roots[3]));
        assert_eq!(range[3], (6, roots[6]));
    }
}
