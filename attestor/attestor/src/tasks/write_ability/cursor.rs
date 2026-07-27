//! Durable Outbox scan cursor (`last_seen` block) for the message-attestation listener.
//!
//! The listener's scan position otherwise lives only in memory, so a restart either **skips** every
//! message published while the node was down (default config, seeds `last_seen` from the current
//! head) or **re-scans and re-signs the entire history** from `start_block` on every boot. This
//! module persists the cursor to a small JSON file so a restart resumes from where it left off.
//!
//! Attestor scan state is deliberately kept **off-chain** (local disk): no attestor-internal
//! bookkeeping belongs on-chain. For the guarantee to hold across pod restarts the file must live on
//! a persistent volume — a deployment concern handled separately from this code.
//!
//! Semantics are **at-least-once**: the cursor is saved once a poll's scanned range has been handed
//! to the signing pipeline, not once each vote is durably gossiped. A crash between the save and the
//! gossip re-scans that range on the next boot, which is harmless — the aggregator dedups by signer
//! and the relayer dedups votes (see [`super::listener::scan_range`]). We therefore never advance the
//! persisted cursor *past* work that has not been scanned, but we may repeat a little already-scanned
//! work; that is the safe direction for a signer.

use std::path::{Path, PathBuf};

use alloy::primitives::Address;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// On-disk cursor record.
///
/// `outbox` guards against resuming a stale cursor against a *different* Outbox (e.g. a
/// re-registration): a mismatch discards the persisted position rather than risk silently skipping
/// the new Outbox's early messages. `chain_key` is recorded for operator legibility and to
/// cross-check the file's provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CursorRecord {
    /// Highest Creditcoin L1 block fully scanned for `MessagePublished` (inclusive).
    last_seen_block: u64,
    /// Outbox address the cursor was advanced against.
    outbox: Address,
    /// Write-ability chain key this cursor belongs to.
    chain_key: u64,
}

/// Verify the write-ability state directory exists (creating it if needed) and is writable, by
/// creating and immediately removing a probe file. Called once at boot so a missing or read-only
/// state volume fails the startup loudly — durable storage is mandatory for a write-ability
/// attestor, and silently degrading to no persistence is the exact footgun the cursor exists to
/// close.
pub fn ensure_writable(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("create write-ability state dir {}", dir.display()))?;
    let probe = dir.join(format!(".write-probe-{}", std::process::id()));
    std::fs::write(&probe, b"ok").with_context(|| {
        format!(
            "write-ability state dir {} is not writable — it must be a writable persistent volume",
            dir.display()
        )
    })?;
    // Best-effort cleanup; a leftover probe file is harmless and will be overwritten next boot.
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

/// Persists the listener's `last_seen` scan cursor to a JSON file, atomically.
///
/// Construct one per (data dir, chain key, resolved Outbox); [`load`](Self::load) reads the last
/// durable position and [`save`](Self::save) advances it.
#[derive(Clone, Debug)]
pub struct CursorStore {
    path: PathBuf,
    outbox: Address,
    chain_key: u64,
}

impl CursorStore {
    /// Cursor file for `chain_key`'s Outbox inside `dir`. The file name is chain-key-scoped so
    /// several attestor instances that happen to share a data dir do not clobber each other's
    /// cursors.
    #[must_use]
    pub fn new(dir: &Path, chain_key: u64, outbox: Address) -> Self {
        Self {
            path: dir.join(format!("outbox-cursor-{chain_key}.json")),
            outbox,
            chain_key,
        }
    }

    /// The file this store reads and writes (for logging).
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load the persisted `last_seen`, or `None` when there is no usable cursor yet:
    ///   - file missing (first boot),
    ///   - unreadable / corrupt (logged, then treated as absent so a bad file never wedges boot),
    ///   - written against a *different* Outbox (stale — discarded to avoid skipping new messages).
    ///
    /// Returning `None` in every failure mode is deliberate: a missing or unusable cursor falls back
    /// to the configured `start_block`/head path, which is always safe (at worst a re-scan), whereas
    /// failing the boot on a bad cursor file would be a needless liveness hazard.
    #[must_use]
    pub fn load(&self) -> Option<u64> {
        let raw = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => {
                tracing::warn!(
                    path = %self.path.display(), %e,
                    "could not read Outbox cursor file; starting from configured start_block/head"
                );
                return None;
            }
        };
        let record: CursorRecord = match serde_json::from_slice(&raw) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    path = %self.path.display(), %e,
                    "Outbox cursor file is corrupt; ignoring it"
                );
                return None;
            }
        };
        if record.outbox != self.outbox {
            tracing::warn!(
                path = %self.path.display(),
                persisted = %record.outbox,
                resolved = %self.outbox,
                "persisted Outbox cursor is for a different Outbox; ignoring it"
            );
            return None;
        }
        Some(record.last_seen_block)
    }

    /// Persist `last_seen` atomically: write to a temp file in the same directory, `fsync` it, then
    /// rename over the target. Rename is atomic within a filesystem, so a crash mid-write can never
    /// leave a torn cursor — a reader sees either the old value or the new one, never a partial file.
    pub fn save(&self, last_seen: u64) -> Result<()> {
        let record = CursorRecord {
            last_seen_block: last_seen,
            outbox: self.outbox,
            chain_key: self.chain_key,
        };
        let json = serde_json::to_vec_pretty(&record).context("serialize Outbox cursor")?;

        let dir = self.path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(dir)
            .with_context(|| format!("create cursor dir {}", dir.display()))?;

        // Temp file in the *same* dir so the final rename stays on one filesystem (cross-device
        // rename is not atomic and would `EXDEV`). Unique per-pid suffix avoids two writers racing on
        // a shared temp name.
        let tmp = self.path.with_extension(format!("json.tmp.{}", std::process::id()));
        {
            use std::io::Write as _;
            let mut f = std::fs::File::create(&tmp)
                .with_context(|| format!("create temp cursor file {}", tmp.display()))?;
            f.write_all(&json).context("write temp cursor file")?;
            // fsync the file contents before the rename so a crash right after the rename can't
            // surface a renamed-but-empty file (rename orders the dir entry, not the data).
            f.sync_all().context("fsync temp cursor file")?;
        }
        std::fs::rename(&tmp, &self.path).with_context(|| {
            format!("rename {} -> {}", tmp.display(), self.path.display())
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    /// A throwaway unique directory under the system temp dir (avoids a `tempfile` dev-dep).
    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let p = std::env::temp_dir().join(format!(
                "attestor-cursor-test-{tag}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn outbox() -> Address {
        address!("00000000000000000000000000000000000000aa")
    }
    fn other() -> Address {
        address!("00000000000000000000000000000000000000bb")
    }

    #[test]
    fn missing_file_loads_none() {
        let dir = TmpDir::new("missing");
        let store = CursorStore::new(dir.path(), 7, outbox());
        assert_eq!(store.load(), None);
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = TmpDir::new("roundtrip");
        let store = CursorStore::new(dir.path(), 7, outbox());
        store.save(12_345).unwrap();
        assert_eq!(store.load(), Some(12_345));
        // A later save overwrites in place (atomic replace), and the new value reads back.
        store.save(20_000).unwrap();
        assert_eq!(store.load(), Some(20_000));
    }

    #[test]
    fn corrupt_file_loads_none() {
        let dir = TmpDir::new("corrupt");
        let store = CursorStore::new(dir.path(), 7, outbox());
        std::fs::write(store.path(), b"not json {{{").unwrap();
        assert_eq!(store.load(), None);
    }

    #[test]
    fn different_outbox_is_ignored() {
        let dir = TmpDir::new("outbox");
        // Written against `outbox()`...
        CursorStore::new(dir.path(), 7, outbox()).save(999).unwrap();
        // ...read back by a store resolved to a *different* Outbox → treated as absent.
        let store = CursorStore::new(dir.path(), 7, other());
        assert_eq!(store.load(), None);
    }

    #[test]
    fn save_creates_missing_dir() {
        let dir = TmpDir::new("mkdir");
        let nested = dir.path().join("a").join("b");
        let store = CursorStore::new(&nested, 1, outbox());
        store.save(42).unwrap();
        assert_eq!(store.load(), Some(42));
    }

    #[test]
    fn ensure_writable_creates_and_probes() {
        let dir = TmpDir::new("probe");
        let nested = dir.path().join("state");
        ensure_writable(&nested).expect("newly created dir must be writable");
        assert!(nested.is_dir());
        // No probe file should linger after the check.
        let leftover: Vec<_> = std::fs::read_dir(&nested).unwrap().collect();
        assert!(leftover.is_empty(), "probe file must be cleaned up");
    }
}
