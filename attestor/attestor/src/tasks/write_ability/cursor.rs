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
//!
//! [`FactoryScanCursorStore`] persists the *other* scan this task runs: `OutboxCreated` discovery
//! against the factory contract (see [`super::resolver::OutboxDiscoveryCursor`]). Without it, a
//! restart rescans that factory's full log history from genesis before write-ability can activate
//! at all; a factory change (governance rotation) instead resumes from the checkpoint already on
//! disk unless `Config::resume_rotation_from_checkpoint` is turned off — the same at-least-once
//! safety applies either way, since re-scanning already-covered blocks only wastes time, never
//! correctness. The record also carries the *floor* each scan began at, which is what lets the
//! resolver's genesis fallback recover a scan that resumed above the event it was looking for
//! (see [`super::resolver`]) even across a restart.

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
    /// Cursor file for `chain_key`'s Outbox inside `dir`.
    ///
    /// **Invariant: each attestor instance must own its `dir`.** The file name is scoped only by
    /// `chain_key`, so two attestors pointed at the *same* `dir` for the same chain write the same
    /// file and clobber each other's `last_seen` — a faster peer's higher value could make a slower
    /// peer skip messages on restart. Production gives every attestor its own persistent volume, so
    /// they never share `dir`. Two safety nets bound the blast radius if the invariant is ever
    /// violated: the `outbox`-mismatch guard in [`load`](Self::load) discards a cursor written
    /// against a *different* Outbox, and the listener rewinds a resumed cursor by a lookback
    /// (`CURSOR_RESUME_LOOKBACK_BLOCKS`) so a cursor left *ahead* re-scans that window rather than
    /// skipping it (downstream dedups the re-signed votes).
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
        let tmp = self
            .path
            .with_extension(format!("json.tmp.{}", std::process::id()));
        {
            use std::io::Write as _;
            let mut f = std::fs::File::create(&tmp)
                .with_context(|| format!("create temp cursor file {}", tmp.display()))?;
            f.write_all(&json).context("write temp cursor file")?;
            // fsync the file contents before the rename so a crash right after the rename can't
            // surface a renamed-but-empty file (rename orders the dir entry, not the data).
            f.sync_all().context("fsync temp cursor file")?;
        }
        std::fs::rename(&tmp, &self.path)
            .with_context(|| format!("rename {} -> {}", tmp.display(), self.path.display()))?;
        Ok(())
    }
}

/// On-disk record of an [`super::resolver::OutboxDiscoveryCursor`]'s scan progress.
///
/// Unlike [`CursorRecord`], `factory` is not a validity gate enforced by this store — the factory
/// is exactly what the scan is trying to pin down, so there is nothing to compare it against at
/// load time. Instead [`FactoryScanCursorStore::load`] returns the record as-is and
/// `resolver::resolve_outbox_address`'s existing `cursor.factory != factory` comparison against the
/// freshly-read on-chain registration discards it, the same way it already discards a mid-run
/// rotation.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FactoryScanRecord {
    /// Highest block scanned so far for `OutboxCreated` on `factory` (inclusive).
    scanned_to: u64,
    /// Factory this scan was performed against.
    factory: Address,
    /// Write-ability chain key this cursor belongs to.
    chain_key: u64,
    /// The latest `OutboxCreated` match found so far, if any: its Outbox address and the block it
    /// was emitted in (see `resolver::OutboxDiscoveryCursor::found`).
    #[serde(skip_serializing_if = "Option::is_none")]
    found_outbox: Option<Address>,
    #[serde(skip_serializing_if = "Option::is_none")]
    found_created_at_block: Option<u64>,
    /// Block this factory's scan *started* from — the floor below which it has never looked. Read
    /// back by the resolver's genesis fallback to tell "scanned everything and this factory has no
    /// Outbox" from "resumed above the event and would never have seen it".
    ///
    /// `Option` purely for backward compatibility with records written before this field existed:
    /// [`FactoryScanCursorStore::load`] maps `None` to `scanned_to`, the conservative reading
    /// ("assume the scan began where it currently sits"), which grants such a record exactly one
    /// genesis fallback. Reading it as 0 instead would silently deny the fallback to precisely the
    /// cursors most likely to need it — one written by a rotation on an older build.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scan_floor: Option<u64>,
}

/// In-memory shape of a [`FactoryScanCursorStore`] record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistedFactoryScan {
    pub factory: Address,
    pub scanned_to: u64,
    pub found: Option<(Address, Option<u64>)>,
    /// Block this factory's scan started from; see [`FactoryScanRecord::scan_floor`].
    pub scan_floor: u64,
}

/// Persists an [`super::resolver::OutboxDiscoveryCursor`]'s `OutboxCreated` discovery progress to a
/// JSON file, atomically.
///
/// Scoped only by `chain_key`, not by factory: the resolved factory can change (governance
/// re-registration), and it is exactly the discovery scan's job to find the Outbox for whichever
/// factory is currently registered — see [`FactoryScanRecord`]'s docs for how a stale factory is
/// handled.
#[derive(Clone, Debug)]
pub struct FactoryScanCursorStore {
    path: PathBuf,
    chain_key: u64,
}

impl FactoryScanCursorStore {
    /// Factory-scan cursor file for `chain_key` inside `dir`. See [`CursorStore::new`]'s
    /// single-owner-of-`dir` invariant — the same one applies here.
    #[must_use]
    pub fn new(dir: &Path, chain_key: u64) -> Self {
        Self {
            path: dir.join(format!("factory-scan-cursor-{chain_key}.json")),
            chain_key,
        }
    }

    /// The file this store reads and writes (for logging).
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load the persisted scan progress, or `None` when there is nothing usable yet: file missing
    /// (first boot), or unreadable/corrupt (logged, then treated as absent so a bad file never
    /// wedges boot — same reasoning as [`CursorStore::load`]).
    #[must_use]
    pub fn load(&self) -> Option<PersistedFactoryScan> {
        let raw = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => {
                tracing::warn!(
                    path = %self.path.display(), %e,
                    "could not read factory-scan cursor file; starting OutboxCreated discovery from the configured factory-scan genesis block"
                );
                return None;
            }
        };
        let record: FactoryScanRecord = match serde_json::from_slice(&raw) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    path = %self.path.display(), %e,
                    "factory-scan cursor file is corrupt; ignoring it"
                );
                return None;
            }
        };
        Some(PersistedFactoryScan {
            factory: record.factory,
            scanned_to: record.scanned_to,
            found: record
                .found_outbox
                .map(|outbox| (outbox, record.found_created_at_block)),
            // Pre-`scan_floor` record: assume the scan began where it now sits, so the resolver
            // grants it one genesis fallback rather than trusting a floor it never recorded.
            scan_floor: record.scan_floor.unwrap_or(record.scanned_to),
        })
    }

    /// Persist `scan` atomically (temp file + fsync + rename — same scheme as
    /// [`CursorStore::save`]). Overwrites whatever was previously recorded for this `chain_key`
    /// outright: every call carries the scan's full current progress, so there is nothing from a
    /// prior record worth preserving.
    pub fn save(&self, scan: &PersistedFactoryScan) -> Result<()> {
        let (found_outbox, found_created_at_block) = match scan.found {
            Some((outbox, block)) => (Some(outbox), block),
            None => (None, None),
        };
        let record = FactoryScanRecord {
            scanned_to: scan.scanned_to,
            factory: scan.factory,
            chain_key: self.chain_key,
            found_outbox,
            found_created_at_block,
            scan_floor: Some(scan.scan_floor),
        };
        let json = serde_json::to_vec_pretty(&record).context("serialize factory-scan cursor")?;

        let dir = self.path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(dir)
            .with_context(|| format!("create cursor dir {}", dir.display()))?;

        let tmp = self
            .path
            .with_extension(format!("json.tmp.{}", std::process::id()));
        {
            use std::io::Write as _;
            let mut f = std::fs::File::create(&tmp).with_context(|| {
                format!("create temp factory-scan cursor file {}", tmp.display())
            })?;
            f.write_all(&json)
                .context("write temp factory-scan cursor file")?;
            f.sync_all()
                .context("fsync temp factory-scan cursor file")?;
        }
        std::fs::rename(&tmp, &self.path)
            .with_context(|| format!("rename {} -> {}", tmp.display(), self.path.display()))?;
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

    fn factory() -> Address {
        address!("00000000000000000000000000000000000000ff")
    }

    #[test]
    fn factory_scan_missing_file_loads_none() {
        let dir = TmpDir::new("factory-missing");
        let store = FactoryScanCursorStore::new(dir.path(), 2);
        assert_eq!(store.load(), None);
    }

    #[test]
    fn factory_scan_round_trips_with_a_winner() {
        let dir = TmpDir::new("factory-roundtrip");
        let store = FactoryScanCursorStore::new(dir.path(), 2);
        let scan = PersistedFactoryScan {
            factory: factory(),
            scanned_to: 12_345,
            found: Some((outbox(), Some(100))),
            scan_floor: 0,
        };
        store.save(&scan).unwrap();
        assert_eq!(store.load(), Some(scan));

        // A later save overwrites in place.
        let advanced = PersistedFactoryScan {
            factory: factory(),
            scanned_to: 20_000,
            found: Some((outbox(), Some(100))),
            scan_floor: 0,
        };
        store.save(&advanced).unwrap();
        assert_eq!(store.load(), Some(advanced));
    }

    /// A scan that has covered some of the range but found no `OutboxCreated` match yet round-trips
    /// with `found: None` rather than a partially-populated pair.
    #[test]
    fn factory_scan_round_trips_with_no_winner_yet() {
        let dir = TmpDir::new("factory-no-winner");
        let store = FactoryScanCursorStore::new(dir.path(), 2);
        let scan = PersistedFactoryScan {
            factory: factory(),
            scanned_to: 2_000,
            found: None,
            scan_floor: 0,
        };
        store.save(&scan).unwrap();
        assert_eq!(store.load(), Some(scan));
    }

    /// `scan_floor` round-trips, so the resolver can tell "scanned everything below" from
    /// "resumed above the event" across a restart — the distinction its genesis fallback turns on.
    #[test]
    fn factory_scan_round_trips_the_scan_floor() {
        let dir = TmpDir::new("factory-floor");
        let store = FactoryScanCursorStore::new(dir.path(), 2);
        let scan = PersistedFactoryScan {
            factory: factory(),
            scanned_to: 705_600,
            found: None,
            scan_floor: 705_530,
        };
        store.save(&scan).unwrap();
        assert_eq!(store.load(), Some(scan));
    }

    /// A record written before `scan_floor` existed loads with the floor set to `scanned_to`, not
    /// to 0. That is the conservative reading — "assume the scan began where it now sits" — and it
    /// grants such a record exactly one genesis fallback. Defaulting to 0 would instead claim the
    /// range below was already covered, denying the recovery to precisely the cursors most likely
    /// to need it: one written by a rotation on a build that predates this field.
    #[test]
    fn factory_scan_record_without_a_floor_defaults_to_its_position() {
        let dir = TmpDir::new("factory-legacy");
        let store = FactoryScanCursorStore::new(dir.path(), 2);
        let legacy = format!(
            r#"{{"scanned_to":705600,"factory":"{}","chain_key":2}}"#,
            factory()
        );
        std::fs::write(store.path(), legacy).unwrap();
        let loaded = store.load().expect("legacy record must still load");
        assert_eq!(loaded.scanned_to, 705_600);
        assert_eq!(loaded.scan_floor, 705_600);
        assert_eq!(loaded.found, None);
    }

    #[test]
    fn factory_scan_corrupt_file_loads_none() {
        let dir = TmpDir::new("factory-corrupt");
        let store = FactoryScanCursorStore::new(dir.path(), 2);
        std::fs::write(store.path(), b"not json {{{").unwrap();
        assert_eq!(store.load(), None);
    }

    /// Unlike [`CursorStore`], loading does not itself discard a record for a different factory —
    /// that discard happens one layer up, in `resolver::resolve_outbox_address`'s comparison
    /// against the freshly-read on-chain factory. This store just returns whatever was persisted.
    #[test]
    fn factory_scan_load_returns_record_regardless_of_which_factory_it_names() {
        let dir = TmpDir::new("factory-any");
        let store = FactoryScanCursorStore::new(dir.path(), 2);
        let old_factory = address!("00000000000000000000000000000000000000ee");
        let scan = PersistedFactoryScan {
            factory: old_factory,
            scanned_to: 500,
            found: None,
            scan_floor: 0,
        };
        store.save(&scan).unwrap();
        assert_eq!(store.load(), Some(scan));
    }

    #[test]
    fn factory_scan_save_creates_missing_dir() {
        let dir = TmpDir::new("factory-mkdir");
        let nested = dir.path().join("a").join("b");
        let store = FactoryScanCursorStore::new(&nested, 1);
        let scan = PersistedFactoryScan {
            factory: factory(),
            scanned_to: 42,
            found: None,
            scan_floor: 0,
        };
        store.save(&scan).unwrap();
        assert_eq!(store.load(), Some(scan));
    }
}
