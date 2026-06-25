//! Sled-backed root store.
//!
//! Schema: key = block height (u64 big-endian, 8 bytes), value = merkle root (H256, 32 bytes)
//! followed by the source block hash (H256, 32 bytes) — 64 bytes total.
//! Big-endian keys ensure sled's sorted iteration yields blocks in height order.
//!
//! Backward compatibility: databases written before the block-hash column existed store
//! 32-byte values (root only). Read paths accept both layouts; a legacy 32-byte value is
//! treated as `block_hash = unknown` (H256::zero) and is never reorg-failed purely for the
//! missing hash — only a genuine root mismatch (as before) is a hard failure.
//!
//! A separate `meta` tree holds a persisted entry counter so we avoid the O(n)
//! `db.len()` startup scan that warms sled's page cache with the entire history.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use sp_core::H256;
use thiserror::Error;

/// Errors returned by [`RootStore`] operations that callers may want to discriminate on.
#[derive(Debug, Error)]
pub enum StoreError {
    /// `put_roots` saw an existing entry for `height` with a different root than the
    /// incoming value. Indicates either a canonical replacement (reorg past finalization
    /// lag) or an inconsistent upstream RPC — either way the operator needs to investigate
    /// before continuing, so we surface this as a hard failure rather than overwriting.
    ///
    /// Carries both the root and the block-hash on each side so operators can tell a
    /// pure root divergence apart from a canonical replacement (same root, different
    /// source block hash) discovered across restart/backfill.
    #[error("reorg or inconsistency at block {height}: stored root {stored_root:?} hash {stored_hash:?}, incoming root {incoming_root:?} hash {incoming_hash:?}")]
    ReorgDetected {
        height: u64,
        stored_root: H256,
        stored_hash: H256,
        incoming_root: H256,
        incoming_hash: H256,
    },
}

/// A stored entry: the merkle root plus the source block hash it was derived from.
///
/// `block_hash` is [`H256::zero`] for legacy entries written before the hash column
/// existed (and is treated as "unknown", not as a literal zero hash, by the reorg guard).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredRoot {
    pub root: H256,
    pub block_hash: H256,
}

/// Soft cap for sled's page cache. The 0.34 default is ~1 GiB which is wildly
/// oversized for a 40-byte-per-record workload; cap it so resident memory
/// stays predictable. This is a hint, not a hard limit — sled may still
/// exceed it under bursty write load.
const SLED_CACHE_CAPACITY_BYTES: u64 = 64 * 1024 * 1024;

/// Name of the side tree used for archiver-internal metadata
/// (currently just the entry counter).
const META_TREE: &[u8] = b"__archiver_meta";

/// Key inside [`META_TREE`] that stores the cached entry counter as a u64-BE.
const META_KEY_COUNT: &[u8] = b"count";

/// Thread-safe handle to the root store. Cheap to clone (wraps Arc<sled::Db>).
#[derive(Clone)]
pub struct RootStore {
    db: Arc<sled::Db>,
    meta: sled::Tree,
    /// Cached entry count — avoids O(n) scan on every status request.
    entry_count: Arc<AtomicUsize>,
}

impl RootStore {
    /// Open (or create) the sled database at the given path.
    ///
    /// On open, the entry counter is seeded from a persisted side tree (O(1));
    /// only when the counter is missing (first run after upgrade, or a freshly
    /// created database) do we fall back to a one-time `db.len()` scan and
    /// persist the result. This avoids re-warming sled's page cache with the
    /// full history on every restart.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let db = sled::Config::default()
            .path(path.as_ref())
            .cache_capacity(SLED_CACHE_CAPACITY_BYTES)
            .open()
            .with_context(|| format!("failed to open sled database at {:?}", path.as_ref()))?;

        let meta = db
            .open_tree(META_TREE)
            .context("failed to open archiver meta tree")?;

        // The default tree (where roots live) is `db` itself, since `Db: Deref<Target=Tree>`.
        let initial_count = match meta.get(META_KEY_COUNT)? {
            Some(bytes) => {
                let arr: [u8; 8] = bytes
                    .as_ref()
                    .try_into()
                    .context("meta count value has wrong length (expected 8 bytes)")?;
                u64::from_be_bytes(arr) as usize
            }
            None => {
                // First run after upgrade, or fresh database. Pay the O(n) cost
                // exactly once and persist the result so subsequent restarts
                // don't have to repeat it.
                let scanned = db.len();
                meta.insert(META_KEY_COUNT, &(scanned as u64).to_be_bytes())?;
                scanned
            }
        };

        Ok(Self {
            db: Arc::new(db),
            meta,
            entry_count: Arc::new(AtomicUsize::new(initial_count)),
        })
    }

    /// Insert a batch of `(height, root, block_hash)` tuples atomically.
    ///
    /// If a height in `roots` already has a *different* root **or a different source
    /// block hash** stored under it, the whole batch is rejected with [`ReorgDetected`].
    /// Silently overwriting would mask reorgs and RPC inconsistencies — EVM continuity
    /// proofs are keyed on these roots, so an undetected canonical replacement at a given
    /// height would silently corrupt any proof spanning that block. Detecting a *same-root,
    /// different-hash* divergence lets us reconcile canonical replacements across
    /// restart/backfill, not just within a single run.
    ///
    /// Idempotent re-insertion of the same root **and** same hash is allowed (covers
    /// backfill replays). Legacy entries (32-byte, hash unknown) only conflict on the
    /// root: a matching root with a previously-unknown hash is accepted and upgraded to
    /// the 64-byte layout.
    pub fn put_roots(&self, roots: &[(u64, H256, H256)]) -> Result<()> {
        // Reorg / inconsistency guard. Scan first; this is a cheap point-read per entry
        // and means we never run the batch insert with a mixed-conflict payload.
        let mut new_entries = 0_usize;
        for (height, incoming_root, incoming_hash) in roots {
            match self.db.get(height.to_be_bytes())? {
                Some(existing) => {
                    let existing = parse_stored(&existing)?;
                    // Root mismatch is always a hard failure (as before).
                    // Same root but a different *known* block hash is a canonical
                    // replacement at the same height — also a reorg.
                    // A legacy entry (block_hash unknown == zero) never fails on the
                    // hash alone; we accept and upgrade it in place.
                    let hash_conflict =
                        !existing.block_hash.is_zero() && existing.block_hash != *incoming_hash;
                    anyhow::ensure!(
                        existing.root == *incoming_root && !hash_conflict,
                        StoreError::ReorgDetected {
                            height: *height,
                            stored_root: existing.root,
                            stored_hash: existing.block_hash,
                            incoming_root: *incoming_root,
                            incoming_hash: *incoming_hash,
                        }
                    );
                    // Same root (and compatible hash) — idempotent replay or a legacy
                    // upgrade. Either way it's not a new entry, so don't count it.
                }
                None => new_entries += 1,
            }
        }

        let mut batch = sled::Batch::default();
        for (height, root, block_hash) in roots {
            batch.insert(&height.to_be_bytes(), encode_value(*root, *block_hash).as_slice());
        }
        self.db
            .apply_batch(batch)
            .context("failed to apply batch insert")?;
        // Only count entries we actually added — idempotent replays must not double-count.
        let new_total = self
            .entry_count
            .fetch_add(new_entries, Ordering::AcqRel)
            .saturating_add(new_entries);
        // Persist the running count to the meta tree. This is best-effort: it
        // does not need to be atomic with the roots batch — if we crash between
        // the two writes the counter will simply drift by at most one batch on
        // the next restart (and only until the next successful put_roots).
        // Failing to persist the counter must not abort archival.
        if let Err(e) = self
            .meta
            .insert(META_KEY_COUNT, &(new_total as u64).to_be_bytes())
        {
            tracing::warn!(error = %e, "failed to persist entry count to meta tree");
        }
        Ok(())
    }

    /// Get roots for an inclusive block range [from, to].
    /// Returns `(block_number, StoredRoot)` pairs in ascending order.
    pub fn get_range(&self, from: u64, to: u64) -> Result<Vec<(u64, StoredRoot)>> {
        let start = from.to_be_bytes();
        let capacity = (to.saturating_sub(from) + 1) as usize;
        let mut results = Vec::with_capacity(capacity);

        for item in self.db.range(start..=to.to_be_bytes()) {
            let (key, value) = item.context("failed to read from sled")?;
            let height = parse_height(&key)?;
            let stored = parse_stored(&value)?;
            results.push((height, stored));
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
    ///
    /// When `start_height` is `Some`, the range `start_height..first_stored_height - 1`
    /// is also reported as a gap if the database begins at an intermediate height
    /// (e.g. after restoring from a partial snapshot). Without it, `--backfill` could
    /// never recover blocks below the first persisted entry because the gap-finder
    /// used neighbour-pair comparison only and had no anchor on the low side.
    pub fn find_gaps(&self, start_height: Option<u64>) -> Result<Vec<(u64, u64)>> {
        let mut gaps = Vec::new();
        // `expected` seeded from `start_height` makes the pre-first-stored region act
        // like any other neighbour-pair gap.
        let mut expected: Option<u64> = start_height;

        for item in self.db.iter() {
            let (key, _) = item.context("failed to read from sled")?;
            let height = parse_height(&key)?;

            if let Some(exp) = expected {
                if height > exp {
                    gaps.push((exp, height - 1));
                }
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

/// Encode a stored value as `root (32 bytes) ++ block_hash (32 bytes)`.
fn encode_value(root: H256, block_hash: H256) -> [u8; 64] {
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(root.as_bytes());
    buf[32..].copy_from_slice(block_hash.as_bytes());
    buf
}

/// Parse a stored value, accepting both the current 64-byte layout
/// (`root ++ block_hash`) and the legacy 32-byte layout (`root` only).
///
/// Legacy values yield `block_hash = H256::zero` ("unknown"); the reorg guard
/// treats a zero stored hash as unknown and never fails on it alone.
fn parse_stored(value: &sled::IVec) -> Result<StoredRoot> {
    match value.len() {
        64 => Ok(StoredRoot {
            root: H256::from_slice(&value[..32]),
            block_hash: H256::from_slice(&value[32..]),
        }),
        32 => {
            tracing::debug!("reading legacy 32-byte value (block hash unknown)");
            Ok(StoredRoot {
                root: H256::from_slice(value.as_ref()),
                block_hash: H256::zero(),
            })
        }
        other => anyhow::bail!("invalid stored value length: expected 32 or 64, got {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test helper: build a `(height, root, block_hash)` tuple with a random hash.
    fn entry(height: u64, root: H256) -> (u64, H256, H256) {
        (height, root, H256::random())
    }

    #[test]
    fn roundtrip_put_get() {
        let dir = tempfile::tempdir().unwrap();
        let store = RootStore::open(dir.path().join("test.sled")).unwrap();

        let root = H256::random();
        let hash = H256::random();
        store.put_roots(&[(42, root, hash)]).unwrap();

        let range = store.get_range(42, 42).unwrap();
        assert_eq!(range.len(), 1);
        assert_eq!(
            range[0],
            (
                42,
                StoredRoot {
                    root,
                    block_hash: hash
                }
            )
        );
        assert_eq!(store.get_range(43, 43).unwrap().len(), 0);
        assert_eq!(store.latest_height().unwrap(), Some(42));
    }

    #[test]
    fn range_query() {
        let dir = tempfile::tempdir().unwrap();
        let store = RootStore::open(dir.path().join("test.sled")).unwrap();

        let roots: Vec<H256> = (0..10).map(|_| H256::random()).collect();
        let entries: Vec<(u64, H256, H256)> = roots
            .iter()
            .enumerate()
            .map(|(i, r)| entry(i as u64, *r))
            .collect();
        store.put_roots(&entries).unwrap();

        let range = store.get_range(3, 6).unwrap();
        assert_eq!(range.len(), 4);
        assert_eq!(range[0].0, 3);
        assert_eq!(range[0].1.root, roots[3]);
        assert_eq!(range[3].0, 6);
        assert_eq!(range[3].1.root, roots[6]);
    }

    #[test]
    fn count_persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sled");

        {
            let store = RootStore::open(&path).unwrap();
            let entries: Vec<(u64, H256, H256)> =
                (0..7).map(|i| entry(i, H256::random())).collect();
            store.put_roots(&entries).unwrap();
            assert_eq!(store.count(), 7);
            // Drop the store, closing the database.
        }

        let reopened = RootStore::open(&path).unwrap();
        // Count must be restored from the meta tree without a full scan.
        assert_eq!(reopened.count(), 7);

        reopened.put_roots(&[entry(7, H256::random())]).unwrap();
        assert_eq!(reopened.count(), 8);
    }

    #[test]
    fn count_recovers_from_missing_meta() {
        // Simulate the upgrade path: a database that pre-exists without the
        // meta tree entry. The first open should one-time-scan and persist.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sled");

        {
            // Open with the new code, write some entries, then manually wipe
            // the meta key to mimic a pre-upgrade db.
            let store = RootStore::open(&path).unwrap();
            let entries: Vec<(u64, H256, H256)> =
                (0..5).map(|i| entry(i, H256::random())).collect();
            store.put_roots(&entries).unwrap();
            store.meta.remove(META_KEY_COUNT).unwrap();
            store.db.flush().unwrap();
        }

        let reopened = RootStore::open(&path).unwrap();
        // The one-time fallback scan should have recovered the count.
        assert_eq!(reopened.count(), 5);
        // And persisted it for next time.
        assert!(reopened.meta.get(META_KEY_COUNT).unwrap().is_some());
    }

    #[test]
    fn put_roots_rejects_conflicting_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let store = RootStore::open(dir.path().join("test.sled")).unwrap();

        let original = H256::from_slice(&[0xaa; 32]);
        let hash = H256::from_slice(&[0x11; 32]);
        store.put_roots(&[(100, original, hash)]).unwrap();

        // Replaying the same root + hash must succeed and stay idempotent.
        store.put_roots(&[(100, original, hash)]).unwrap();
        assert_eq!(
            store.get_range(100, 100).unwrap(),
            vec![(
                100,
                StoredRoot {
                    root: original,
                    block_hash: hash
                }
            )]
        );

        // A different root for the same height is a reorg / inconsistency signal —
        // surface it instead of silently overwriting.
        let conflicting = H256::from_slice(&[0xbb; 32]);
        let err = store
            .put_roots(&[(100, conflicting, hash)])
            .expect_err("conflicting overwrite must be rejected");
        let downcast = err
            .downcast_ref::<StoreError>()
            .expect("StoreError variant");
        assert!(matches!(
            downcast,
            StoreError::ReorgDetected { height: 100, .. }
        ));

        // Storage stays at the original root + hash.
        assert_eq!(
            store.get_range(100, 100).unwrap(),
            vec![(
                100,
                StoredRoot {
                    root: original,
                    block_hash: hash
                }
            )]
        );
    }

    #[test]
    fn put_roots_rejects_same_root_different_block_hash() {
        // Canonical replacement at the same height: the merkle root happens to match
        // but the source block hash differs. This is a reorg that #1088's root-only
        // guard could miss — the block-hash column lets us catch it across restarts.
        let dir = tempfile::tempdir().unwrap();
        let store = RootStore::open(dir.path().join("test.sled")).unwrap();

        let root = H256::from_slice(&[0xaa; 32]);
        let hash_a = H256::from_slice(&[0x11; 32]);
        let hash_b = H256::from_slice(&[0x22; 32]);
        store.put_roots(&[(100, root, hash_a)]).unwrap();

        let err = store
            .put_roots(&[(100, root, hash_b)])
            .expect_err("same root, different block hash must be rejected");
        let downcast = err
            .downcast_ref::<StoreError>()
            .expect("StoreError variant");
        assert!(matches!(
            downcast,
            StoreError::ReorgDetected {
                height: 100,
                stored_hash,
                incoming_hash,
                ..
            } if *stored_hash == hash_a && *incoming_hash == hash_b
        ));

        // Storage is unchanged.
        assert_eq!(
            store.get_range(100, 100).unwrap(),
            vec![(
                100,
                StoredRoot {
                    root,
                    block_hash: hash_a
                }
            )]
        );
    }

    #[test]
    fn put_roots_accepts_legacy_value_upgrade() {
        // A legacy 32-byte value (root only, block hash unknown) must be accepted and
        // upgraded to the 64-byte layout on a matching-root replay, without a reorg
        // failure on the (previously unknown) hash.
        let dir = tempfile::tempdir().unwrap();
        let store = RootStore::open(dir.path().join("test.sled")).unwrap();

        let root = H256::from_slice(&[0xcd; 32]);
        // Write a legacy 32-byte value directly, bypassing the encoder.
        store
            .db
            .insert(100u64.to_be_bytes(), root.as_bytes())
            .unwrap();

        // Legacy value parses with a zero (unknown) block hash.
        assert_eq!(
            store.get_range(100, 100).unwrap(),
            vec![(
                100,
                StoredRoot {
                    root,
                    block_hash: H256::zero()
                }
            )]
        );

        // Replaying the same root with a now-known hash must succeed (upgrade in place)
        // rather than reorg-fail on the previously-unknown hash.
        let hash = H256::from_slice(&[0x33; 32]);
        store.put_roots(&[(100, root, hash)]).unwrap();
        assert_eq!(
            store.get_range(100, 100).unwrap(),
            vec![(
                100,
                StoredRoot {
                    root,
                    block_hash: hash
                }
            )]
        );

        // But a legacy entry with a *different* root still fails (root guard intact).
        let other = H256::from_slice(&[0xee; 32]);
        store
            .db
            .insert(200u64.to_be_bytes(), other.as_bytes())
            .unwrap();
        let err = store
            .put_roots(&[(200, root, hash)])
            .expect_err("legacy root mismatch must still be rejected");
        assert!(err.downcast_ref::<StoreError>().is_some());
    }

    #[test]
    fn put_roots_idempotent_replay_does_not_double_count() {
        let dir = tempfile::tempdir().unwrap();
        let store = RootStore::open(dir.path().join("test.sled")).unwrap();

        let entries: Vec<(u64, H256, H256)> = (0..5).map(|i| entry(i, H256::random())).collect();
        store.put_roots(&entries).unwrap();
        assert_eq!(store.count(), 5);

        // Re-applying the same batch must not bump the counter (idempotent replay
        // covers backfill / restart edge cases).
        store.put_roots(&entries).unwrap();
        assert_eq!(store.count(), 5);
    }

    #[test]
    fn find_gaps_reports_pre_first_stored_gap() {
        let dir = tempfile::tempdir().unwrap();
        let store = RootStore::open(dir.path().join("test.sled")).unwrap();

        // DB begins at an intermediate height (mimicking a partial snapshot restore).
        store
            .put_roots(&[entry(50, H256::random()), entry(51, H256::random())])
            .unwrap();

        // Without an anchor, only neighbour-pair gaps are visible — the pre-first
        // region is invisible.
        let no_anchor = store.find_gaps(None).unwrap();
        assert!(no_anchor.is_empty());

        // With `start_height = 10`, the range 10..=49 should now be reported.
        let with_anchor = store.find_gaps(Some(10)).unwrap();
        assert_eq!(with_anchor, vec![(10, 49)]);
    }

    #[test]
    fn find_gaps_mid_range_still_reported_with_anchor() {
        let dir = tempfile::tempdir().unwrap();
        let store = RootStore::open(dir.path().join("test.sled")).unwrap();

        store
            .put_roots(&[
                entry(10, H256::random()),
                entry(11, H256::random()),
                entry(15, H256::random()),
                entry(20, H256::random()),
            ])
            .unwrap();

        let gaps = store.find_gaps(Some(5)).unwrap();
        assert_eq!(gaps, vec![(5, 9), (12, 14), (16, 19)]);
    }
}
