//! Persistent per-watcher block cursors.
//!
//! Both the Outbox watcher (source chain `MessagePublished`) and the acknowledgment watcher
//! (destination chain `MessageDelivered`) scan chain logs by block range. Without persistence they
//! start from the chain head on every boot, so any event emitted while the relayer was down is
//! silently skipped. This store records the last block each watcher has fully processed and is
//! consulted on startup, so a restart resumes from `last_processed + 1` instead of the head — the
//! relayer never misses an on-chain event, even across downtime.
//!
//! Storage is a single JSON file (`{ "outbox:2": 1234, "ack:2": 5678 }`) written atomically
//! (temp file + rename) so a crash mid-write cannot corrupt it. Reprocessing the tail of a range
//! after an unclean shutdown is safe: delivery is idempotent (`MessageAlreadyValidated`) and acks
//! are deduped + idempotent (`MessageAlreadyAcknowledged`), so the cursor gives at-least-once,
//! never at-most-once.
//!
//! Note: this covers durable *on-chain* events. Attestor votes travel over gossip (ephemeral) and
//! are out of scope here — a relayer that was down while votes were gossiped relies on the votes
//! being re-observed, not on this cursor.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};

/// A JSON-file-backed map of `watcher key -> last fully-processed block`.
#[derive(Debug)]
pub struct CheckpointStore {
    path: PathBuf,
    inner: Mutex<HashMap<String, u64>>,
}

impl CheckpointStore {
    /// Load the store from `path`, treating a missing file as an empty store.
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let inner: HashMap<String, u64> = match std::fs::read_to_string(&path) {
            Ok(text) if text.trim().is_empty() => HashMap::new(),
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("parsing checkpoint file {}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => {
                return Err(e)
                    .with_context(|| format!("reading checkpoint file {}", path.display()))
            }
        };
        Ok(Self {
            path,
            inner: Mutex::new(inner),
        })
    }

    /// The last fully-processed block for `key`, if any has been recorded.
    pub fn get(&self, key: &str) -> Option<u64> {
        self.inner
            .lock()
            .expect("checkpoint mutex")
            .get(key)
            .copied()
    }

    /// Record `block` as the last fully-processed block for `key` and persist the whole store.
    ///
    /// The lock is held across the file write so concurrent watchers cannot interleave a stale
    /// snapshot over a newer one; writes are small and infrequent (one per poll tick).
    pub fn set(&self, key: &str, block: u64) -> Result<()> {
        let mut guard = self.inner.lock().expect("checkpoint mutex");
        guard.insert(key.to_string(), block);
        let serialized =
            serde_json::to_string_pretty(&*guard).context("serializing checkpoint store")?;
        write_atomic(&self.path, serialized.as_bytes())
    }
}

/// Write `bytes` to `path` atomically via a sibling temp file + rename.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating checkpoint dir {}", parent.display()))?;
        }
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)
        .with_context(|| format!("writing checkpoint temp file {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming checkpoint temp file into {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_persists_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cp.json");

        let store = CheckpointStore::load(&path).unwrap();
        assert_eq!(store.get("outbox:2"), None);
        store.set("outbox:2", 100).unwrap();
        store.set("ack:2", 250).unwrap();
        store.set("outbox:2", 150).unwrap(); // overwrite advances

        // A fresh load sees the persisted cursors.
        let reloaded = CheckpointStore::load(&path).unwrap();
        assert_eq!(reloaded.get("outbox:2"), Some(150));
        assert_eq!(reloaded.get("ack:2"), Some(250));
        assert_eq!(reloaded.get("missing"), None);
    }

    #[test]
    fn missing_file_is_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("cp.json");
        let store = CheckpointStore::load(&path).unwrap();
        assert_eq!(store.get("anything"), None);
        // First write creates the nested dir.
        store.set("ack:7", 42).unwrap();
        assert_eq!(CheckpointStore::load(&path).unwrap().get("ack:7"), Some(42));
    }
}
