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
        let mut results = Vec::new();
        for root in self.iter_range(start_height, end_height) {
            results.push(root?);
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
        let source = SledSource::open(&db_path).unwrap();

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
}
