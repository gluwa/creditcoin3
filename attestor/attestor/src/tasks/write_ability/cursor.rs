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
//! Scanned messages are durably journaled before this cursor can advance. The pending journal
//! survives crashes independently of the bounded restart rewind and is replayed until gossip
//! accepts a publish. Downstream delivery remains recoverable through reobservation.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Semaphore;

use alloy::primitives::Address;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// On-disk cursor record.
///
/// `outbox` guards against resuming a stale cursor against a *different* Outbox (e.g. a
/// re-registration): a mismatch rejects the persisted position rather than risk silently skipping
/// the new Outbox's early messages. `chain_key` is recorded for operator legibility and to
/// cross-check the file's provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CursorRecord {
    /// Version marker: false/absent is the legacy single-Outbox scanner.
    #[serde(default)]
    all_outboxes: bool,
    /// Older scanners did not journal pending work; replay their configured history once.
    #[serde(default)]
    durable_pending: bool,
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
    // Keep exclusive ownership until the last cloned disk operation completes.
    directory: Option<Arc<super::storage::StateDirectory>>,
    io: Arc<Semaphore>,
    all_outboxes: bool,
    migration_floor: u64,
    path: PathBuf,
    outbox: Address,
    chain_key: u64,
}

impl CursorStore {
    /// Cursor file for `chain_key`'s Outbox inside `dir`.
    ///
    /// `dir` must be private to an attestor identity. The production listener obtains its cursor
    /// from the pending store, retaining the same identity-directory ownership lock.
    #[must_use]
    pub fn new(dir: &Path, chain_key: u64, outbox: Address) -> Self {
        Self {
            directory: None,
            io: Arc::new(Semaphore::new(1)),
            all_outboxes: false,
            migration_floor: 0,
            path: dir.join(format!("outbox-cursor-{chain_key}.json")),
            outbox,
            chain_key,
        }
    }

    /// A single cursor covers every Outbox because the entire source-block range is scanned and
    /// historically authenticated before it advances. A default/registry change cannot reset it.
    #[must_use]
    pub fn for_all_outboxes(dir: &Path, chain_key: u64, start_block: Option<u64>) -> Self {
        Self {
            all_outboxes: true,
            migration_floor: start_block.unwrap_or(0).saturating_sub(1),
            ..Self::new(dir, chain_key, Address::ZERO)
        }
    }

    pub(super) fn with_directory(
        mut self,
        directory: Option<Arc<super::storage::StateDirectory>>,
    ) -> Self {
        self.directory = directory;
        self
    }

    /// Explicit operator floor used during migration and restart lookback. Zero means no floor.
    pub(super) fn scan_floor(&self) -> u64 {
        self.migration_floor
    }

    /// The file this store reads and writes (for logging).
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Only a genuinely missing file is a fresh start. Corruption, I/O errors and identity
    /// mismatches fail closed so a restart cannot silently skip downtime history.
    pub fn load(&self) -> Result<Option<u64>> {
        let raw = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(err)
                    .context("read Outbox cursor; restore it or configure a safe replay")
            }
        };
        let record: CursorRecord = serde_json::from_slice(&raw).context("invalid Outbox cursor")?;
        anyhow::ensure!(
            record.chain_key == self.chain_key,
            "cursor belongs to another chain"
        );
        if self.all_outboxes && (!record.all_outboxes || !record.durable_pending) {
            return Ok(Some(self.migration_floor));
        }
        anyhow::ensure!(
            record.all_outboxes == self.all_outboxes && record.outbox == self.outbox,
            "cursor belongs to another Outbox/scanner"
        );
        Ok(Some(record.last_seen_block))
    }

    async fn disk<T: Send + 'static>(
        &self,
        work: impl FnOnce(Self) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let store = self.clone();
        blocking_io(store.io.clone(), move || work(store)).await
    }
    pub async fn load_async(&self) -> Result<Option<u64>> {
        self.disk(|store| store.load()).await
    }
    pub async fn save_async(&self, last_seen: u64) -> Result<()> {
        self.disk(move |store| store.save(last_seen)).await
    }

    /// Persist `last_seen` atomically: write to a temp file in the same directory, `fsync` it, then
    /// rename over the target. Rename is atomic within a filesystem, so a crash mid-write can never
    /// leave a torn cursor — a reader sees either the old value or the new one, never a partial file.
    pub fn save(&self, last_seen: u64) -> Result<()> {
        let record = CursorRecord {
            all_outboxes: self.all_outboxes,
            durable_pending: self.all_outboxes,
            last_seen_block: last_seen,
            outbox: self.outbox,
            chain_key: self.chain_key,
        };
        let json = serde_json::to_vec_pretty(&record).context("serialize Outbox cursor")?;

        atomic_write(&self.path, &json)
    }
}

/// Acquire capacity asynchronously, then keep it inside the blocking closure. Cancelling the
/// waiter cannot release serialization while disk work continues, or consume blocking threads
/// waiting on another operation's lock.
pub(super) async fn blocking_io<T: Send + 'static>(
    io: Arc<Semaphore>,
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    let permit = io
        .acquire_owned()
        .await
        .context("persistence worker closed")?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work()
    })
    .await
    .context("persistence disk worker failed")?
}

/// Persist data and its directory entry; readers see the complete old or new record.
/// Callers serialize writes to a path, including across cancellation/rotation.
pub(super) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(tmp, path)?;
    std::fs::File::open(dir)?.sync_all()?;
    Ok(())
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
        assert_eq!(store.load().unwrap(), None);
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = TmpDir::new("roundtrip");
        let store = CursorStore::new(dir.path(), 7, outbox());
        store.save(12_345).unwrap();
        assert_eq!(store.load().unwrap(), Some(12_345));
        // A later save overwrites in place (atomic replace), and the new value reads back.
        store.save(20_000).unwrap();
        assert_eq!(store.load().unwrap(), Some(20_000));
    }

    #[test]
    fn corrupt_file_fails_closed() {
        let dir = TmpDir::new("corrupt");
        let store = CursorStore::new(dir.path(), 7, outbox());
        std::fs::write(store.path(), b"not json {{{").unwrap();
        assert!(store.load().is_err());
    }

    #[test]
    fn different_outbox_fails_closed() {
        let dir = TmpDir::new("outbox");
        // Written against `outbox()`...
        CursorStore::new(dir.path(), 7, outbox()).save(999).unwrap();
        // ...read back by a store resolved to a different Outbox → fail closed.
        let store = CursorStore::new(dir.path(), 7, other());
        assert!(store.load().is_err());
    }

    #[test]
    fn save_creates_missing_dir() {
        let dir = TmpDir::new("mkdir");
        let nested = dir.path().join("a").join("b");
        let store = CursorStore::new(&nested, 1, outbox());
        store.save(42).unwrap();
        assert_eq!(store.load().unwrap(), Some(42));
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
    #[test]
    fn single_outbox_cursor_upgrade_replays_all_history_once() {
        let dir = TmpDir::new("migrate-all");
        CursorStore::new(dir.path(), 7, outbox())
            .save(9000)
            .unwrap();
        let store = CursorStore::for_all_outboxes(dir.path(), 7, None);
        assert_eq!(
            store.load().unwrap(),
            Some(0),
            "old progress did not cover other Outboxes"
        );
        store.save(2000).unwrap();
        assert_eq!(
            CursorStore::for_all_outboxes(dir.path(), 7, None)
                .load()
                .unwrap(),
            Some(2000)
        );
    }

    #[test]
    fn cursor_rejects_wrong_chain_even_when_address_matches() {
        let dir = TmpDir::new("wrong-chain");
        let store = CursorStore::for_all_outboxes(dir.path(), 7, None);
        store.save(9000).unwrap();
        let mut record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(store.path()).unwrap()).unwrap();
        record["chain_key"] = 8.into();
        std::fs::write(store.path(), serde_json::to_vec(&record).unwrap()).unwrap();
        assert!(store.load().is_err());
    }
    #[test]
    fn migration_honors_explicit_start_before_runtime_support() {
        let dir = TmpDir::new("migration-floor");
        CursorStore::new(dir.path(), 7, outbox())
            .save(9000)
            .unwrap();
        let store = CursorStore::for_all_outboxes(dir.path(), 7, Some(4000));
        assert_eq!(store.load().unwrap(), Some(3999));
    }
    #[test]
    fn cursor_without_pending_journal_marker_replays_from_configured_floor() {
        let dir = TmpDir::new("migrate-pending");
        let store = CursorStore::for_all_outboxes(dir.path(), 7, Some(100));
        store.save(2000).unwrap();
        let mut record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(store.path()).unwrap()).unwrap();
        record.as_object_mut().unwrap().remove("durable_pending");
        std::fs::write(store.path(), serde_json::to_vec(&record).unwrap()).unwrap();
        assert_eq!(store.load().unwrap(), Some(99));
    }

    #[tokio::test]
    async fn cancelling_disk_waiter_keeps_writes_serialized() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let io = Arc::new(Semaphore::new(1));
        let finished = Arc::new(AtomicBool::new(false));
        let first_finished = finished.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let first = tokio::spawn(blocking_io(io.clone(), move || {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            first_finished.store(true, Ordering::SeqCst);
            Ok(())
        }));
        started_rx.await.unwrap();
        first.abort();
        assert!(first.await.unwrap_err().is_cancelled());
        let mut second = tokio::spawn(blocking_io(io, move || {
            anyhow::ensure!(
                finished.load(Ordering::SeqCst),
                "second write overtook cancelled first write"
            );
            Ok(())
        }));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut second)
                .await
                .is_err()
        );
        release_tx.send(()).unwrap();
        second.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn cancelling_disk_waiter_keeps_namespace_locked_until_disk_finishes() {
        use std::time::Duration;

        let dir = TmpDir::new("cancelled-namespace-owner");
        let owner = super::super::pending::Store::open(dir.path(), 7, [1; 32], None)
            .await
            .unwrap();
        let cursor = owner.cursor(None);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
        let waiter = tokio::spawn(async move {
            cursor
                .disk(move |store| {
                    started_tx.send(()).unwrap();
                    release_rx
                        .recv_timeout(Duration::from_secs(5))
                        .context("timed out releasing cancelled disk operation")?;
                    store.save(123)?;
                    // Signal only after the blocking operation drops its ownership guard.
                    drop(store);
                    finished_tx.send(()).unwrap();
                    Ok(())
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), started_rx)
            .await
            .unwrap()
            .unwrap();
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        drop(owner);

        assert!(
            super::super::pending::Store::open(dir.path(), 7, [1; 32], None)
                .await
                .is_err()
        );
        release_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), finished_rx)
            .await
            .unwrap()
            .unwrap();

        let reopened = super::super::pending::Store::open(dir.path(), 7, [1; 32], None)
            .await
            .unwrap();
        assert_eq!(reopened.cursor(None).load_async().await.unwrap(), Some(123));
    }
}
