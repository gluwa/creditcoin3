//! Sled database source for reading block root data.
//!
//! The database schema is:
//! - Key: block height as big-endian u64 bytes (8 bytes)
//! - Value: block root digest (32 bytes)

use std::path::Path;

use anyhow::{Context, Result};
use attestor_primitives::Digest;
use tracing::{debug, info};

pub use super::{RootInfo, RootSource};

/// Source for reading block roots from a Sled database.
pub struct SledSource {
    db: sled::Db,
}

impl SledSource {
    /// Open a Sled database at the given path.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let db = sled::open(path.as_ref())
            .with_context(|| format!("Failed to open sled database at {:?}", path.as_ref()))?;

        info!(
            "Opened sled database at {:?} with {} entries",
            path.as_ref(),
            db.len()
        );

        Ok(Self { db })
    }

    /// Verify there are no gaps between `first` and `last` (inclusive) in this database.
    ///
    /// Compares the database's total entry count against the span rather than scanning and
    /// parsing every value (as `get_range` would): since keys are unique big-endian heights and
    /// `first`/`last` are the true min/max keys, `count == span` is sufficient to prove
    /// contiguity, without paying to materialize a `RootInfo` for every entry in a large
    /// database.
    pub(super) fn assert_gapless(&self, first: u64, last: u64) -> Result<()> {
        let expected = last - first + 1;
        let actual = self.db.len() as u64;
        if actual != expected {
            anyhow::bail!(
                "Sled database has {actual} entries but spans heights [{first}, {last}] \
                 (expected {expected}); it has internal gaps or unexpected keys"
            );
        }
        Ok(())
    }
}

impl RootSource for SledSource {
    /// Get a single block root by height.
    fn get(&self, height: u64) -> Result<Option<RootInfo>> {
        let key = height.to_be_bytes();
        match self.db.get(key)? {
            Some(value) => {
                let digest = parse_digest(&value)?;
                Ok(Some(RootInfo { digest, height }))
            }
            None => Ok(None),
        }
    }

    fn get_range(&self, start_height: u64, end_height: u64) -> Result<Vec<RootInfo>> {
        let results: Vec<RootInfo> = self
            .iter_range(start_height, end_height)
            .collect::<Result<Vec<_>>>()?;

        let expected_count = (end_height - start_height + 1) as usize;
        if results.len() != expected_count {
            anyhow::bail!(
                "Sled database has {} entries for range [{start_height}, {end_height}], \
                 expected {expected_count}",
                results.len()
            );
        }
        for (i, entry) in results.iter().enumerate() {
            let expected_height = start_height + i as u64;
            if entry.height != expected_height {
                anyhow::bail!(
                    "Non-contiguous heights in sled database: expected height {expected_height} \
                     at index {i}, got {} (range [{start_height}, {end_height}])",
                    entry.height
                );
            }
        }

        Ok(results)
    }

    /// Get the first (lowest height) entry in the database.
    fn first(&self) -> Result<Option<RootInfo>> {
        match self.db.first()? {
            Some((key, value)) => {
                let height = parse_height(&key)?;
                let digest = parse_digest(&value)?;
                Ok(Some(RootInfo { digest, height }))
            }
            None => Ok(None),
        }
    }

    /// Get the last (highest height) entry in the database.
    fn last(&self) -> Result<Option<RootInfo>> {
        match self.db.last()? {
            Some((key, value)) => {
                let height = parse_height(&key)?;
                let digest = parse_digest(&value)?;
                Ok(Some(RootInfo { digest, height }))
            }
            None => Ok(None),
        }
    }

    /// Iterate over a range of block roots [start_height, end_height] (inclusive).
    ///
    /// Returns an iterator that yields `RootInfo` entries in ascending block height order.
    fn iter_range(
        &self,
        start_height: u64,
        end_height: u64,
    ) -> Box<dyn Iterator<Item = Result<RootInfo>> + '_> {
        let start_key = start_height.to_be_bytes();
        // Use end_height + 1 to make the range inclusive of end_height
        let end_key = (end_height + 1).to_be_bytes();

        Box::new(self.db.range(start_key..end_key).map(|result| {
            let (key, value) = result.context("Failed to read from sled database")?;
            let height = parse_height(&key)?;
            let digest = parse_digest(&value)?;
            debug!("Read block root at height {}: {:?}", height, digest);
            Ok(RootInfo { digest, height })
        }))
    }
}

/// Parse a height from a sled key (big-endian u64).
fn parse_height(key: &sled::IVec) -> Result<u64> {
    let bytes: [u8; 8] = key
        .as_ref()
        .try_into()
        .with_context(|| format!("Invalid key length: expected 8 bytes, got {}", key.len()))?;
    Ok(u64::from_be_bytes(bytes))
}

/// Parse a digest from a sled value (32 bytes).
fn parse_digest(value: &sled::IVec) -> Result<Digest> {
    if value.len() != 32 {
        anyhow::bail!(
            "Invalid digest length: expected 32 bytes, got {}",
            value.len()
        );
    }
    Ok(Digest::from_slice(value.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Open a `SledSource` at `path`, tolerating a lock the previous handle has not
    /// released yet.
    ///
    /// Every test below writes with a plain `sled::Db`, drops it, then reopens the same
    /// path through `SledSource`. Dropping a `sled::Db` does not synchronously release
    /// its file lock: the context is shared with a background flusher thread, so the
    /// `flock` can outlive the `Drop` that closed our handle. When that thread is starved
    /// of CPU the gap is wide enough that an immediate reopen fails, which reads as a
    /// logic failure rather than the scheduling artefact it is.
    ///
    /// `archiver::store` carries the same helper for the same reason; see
    /// `open_after_close` there.
    ///
    /// Bounded deliberately tight, so a genuine open failure (a corrupt database, a bad
    /// path) still surfaces in about a second instead of being buried under a long backoff.
    fn open_after_close(path: &std::path::Path) -> SledSource {
        const ATTEMPTS: usize = 20;
        const WAIT: std::time::Duration = std::time::Duration::from_millis(50);

        let mut last = None;
        for _ in 0..ATTEMPTS {
            match SledSource::open(path) {
                Ok(source) => return source,
                Err(e) => {
                    last = Some(e);
                    std::thread::sleep(WAIT);
                }
            }
        }
        panic!(
            "could not reopen {} after {ATTEMPTS} attempts over {:?}: {:?}",
            path.display(),
            WAIT * ATTEMPTS as u32,
            last.expect("at least one attempt failed"),
        )
    }

    #[test]
    fn test_sled_source_read_write() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test_db");

        // Create a database with some test data
        {
            let db = sled::open(&db_path).unwrap();

            // Insert some test entries
            for height in 0..10u64 {
                let key = height.to_be_bytes();
                let mut digest = [0u8; 32];
                digest[0..8].copy_from_slice(&height.to_be_bytes());
                db.insert(key, &digest[..]).unwrap();
            }
            db.flush().unwrap();
        }

        // Open with SledSource and verify
        let source = open_after_close(&db_path);

        // Test get
        let root = source.get(5).unwrap().unwrap();
        assert_eq!(root.height, 5);

        // Test first/last
        let first = source.first().unwrap().unwrap();
        assert_eq!(first.height, 0);

        let last = source.last().unwrap().unwrap();
        assert_eq!(last.height, 9);

        // Test iter_range (inclusive end)
        let roots: Vec<_> = source.iter_range(2, 5).collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(roots.len(), 4); // [2, 3, 4, 5]
        assert_eq!(roots[0].height, 2);
        assert_eq!(roots[3].height, 5);
    }

    #[test]
    fn test_get_range_gap_returns_error() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test_db");

        {
            let db = sled::open(&db_path).unwrap();
            // Insert heights 0, 1, 2, 4, 5 — block 3 is missing
            for height in [0u64, 1, 2, 4, 5] {
                let key = height.to_be_bytes();
                let mut digest = [0u8; 32];
                digest[0..8].copy_from_slice(&height.to_be_bytes());
                db.insert(key, &digest[..]).unwrap();
            }
            db.flush().unwrap();
        }

        let source = open_after_close(&db_path);
        assert!(
            source.get_range(0, 5).is_err(),
            "expected error for range with gap"
        );
    }

    #[test]
    fn test_assert_gapless_ok() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test_db");

        {
            let db = sled::open(&db_path).unwrap();
            for height in 0u64..10 {
                let key = height.to_be_bytes();
                let digest = [0u8; 32];
                db.insert(key, &digest[..]).unwrap();
            }
            db.flush().unwrap();
        }

        let source = open_after_close(&db_path);
        assert!(source.assert_gapless(0, 9).is_ok());
    }

    #[test]
    fn test_assert_gapless_detects_gap() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test_db");

        {
            let db = sled::open(&db_path).unwrap();
            // heights 0..5 and 7..10, missing 5 and 6
            for height in [0u64, 1, 2, 3, 4, 7, 8, 9] {
                let key = height.to_be_bytes();
                let digest = [0u8; 32];
                db.insert(key, &digest[..]).unwrap();
            }
            db.flush().unwrap();
        }

        let source = open_after_close(&db_path);
        assert!(source.assert_gapless(0, 9).is_err());
    }

    #[test]
    fn test_get_range_missing_end_returns_error() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test_db");

        {
            let db = sled::open(&db_path).unwrap();
            for height in 0u64..5 {
                let key = height.to_be_bytes();
                let mut digest = [0u8; 32];
                digest[0..8].copy_from_slice(&height.to_be_bytes());
                db.insert(key, &digest[..]).unwrap();
            }
            db.flush().unwrap();
        }

        let source = open_after_close(&db_path);
        // Request [0, 9] but only [0, 4] exist
        assert!(
            source.get_range(0, 9).is_err(),
            "expected error when range exceeds available data"
        );
    }
}
