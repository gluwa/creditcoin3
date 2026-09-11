//! Progress / freshness bookkeeping behind `/status` and `/ready`.
//!
//! Liveness (the process answers) and readiness (the archive is current and durable) are
//! deliberately separate. A halted source chain with a fresh head sample and full coverage is
//! healthy; a silently stale subscription with an unchanged height is not, even though both
//! return HTTP 200 on `/status`.
//!
//! Readiness is judged against the *mature target*: the highest block the archiver should hold
//! by now. It is the same number the root stream fetches up to (the attested height clamped to
//! the source head, or the source-resolved mature height), fed in by whoever samples the head,
//! so this module never decides maturity itself.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

/// Sentinel for "never happened" in the millisecond-offset atomics.
const NEVER: u64 = u64::MAX;
/// Sentinel for "nothing is mature yet" in `mature_target`, distinct from [`NEVER`] (unknown).
const NOTHING_MATURE: u64 = u64::MAX - 1;

/// What the archive should hold by now, as last derived from a head sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatureTarget {
    /// Not known yet: no sample so far, or the attested height has not been read from
    /// Creditcoin (or none is published). Not ready: an archive cannot be judged current
    /// against a target nobody has seen.
    Unknown,
    /// Known, and nothing is mature (a fixed lag deeper than the chain): trivially current.
    NothingMature,
    /// The highest block the archive should hold.
    Height(u64),
}

pub struct Health {
    started: Instant,
    pub chain_id: u64,
    /// Human-readable description of what bounds the archive (see [`HealthSnapshot::bound`]).
    pub bound: String,
    ready_lag_blocks: u64,
    stale_after: Duration,

    source_head: AtomicU64,
    source_head_at: AtomicU64,
    /// [`NEVER`] while no target is known, [`NOTHING_MATURE`] when one is known to be empty.
    mature_target: AtomicU64,
    last_progress_height: AtomicU64,
    last_progress_at: AtomicU64,
    last_flush_ok_at: AtomicU64,
    flush_errors: AtomicU64,
    last_flush_error: Mutex<Option<String>>,
    reconnects: AtomicU64,
}

/// What `/status` and `/ready` serialize. Ages are milliseconds; `null` means "never".
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HealthSnapshot {
    pub chain_id: u64,
    /// What the archive is bounded by: the attested height, or a source-resolved maturity.
    pub bound: String,
    pub uptime_ms: u64,
    pub latest_archived_block: Option<u64>,
    pub total_blocks: usize,
    pub source_head: Option<u64>,
    pub source_head_age_ms: Option<u64>,
    /// The highest block the archiver should have by now: the attested height clamped to the
    /// source head, or the source-resolved mature height. `null` until one is known.
    pub mature_target: Option<u64>,
    /// `mature_target - latest_archived_block`, floored at 0.
    pub lag_blocks: Option<u64>,
    pub last_progress_age_ms: Option<u64>,
    pub last_flush_ok_age_ms: Option<u64>,
    pub flush_errors: u64,
    pub last_flush_error: Option<String>,
    pub reconnects: u64,
    pub ready: bool,
    /// Human-readable reasons when `ready` is false; empty otherwise.
    pub not_ready_reasons: Vec<String>,
}

impl Health {
    pub fn new(
        chain_id: u64,
        bound: impl Into<String>,
        ready_lag_blocks: u64,
        stale_after: Duration,
    ) -> Self {
        Self {
            started: Instant::now(),
            chain_id,
            bound: bound.into(),
            ready_lag_blocks,
            stale_after,
            source_head: AtomicU64::new(0),
            source_head_at: AtomicU64::new(NEVER),
            mature_target: AtomicU64::new(NEVER),
            last_progress_height: AtomicU64::new(0),
            last_progress_at: AtomicU64::new(NEVER),
            last_flush_ok_at: AtomicU64::new(NEVER),
            flush_errors: AtomicU64::new(0),
            last_flush_error: Mutex::new(None),
            reconnects: AtomicU64::new(0),
        }
    }

    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    fn age(&self, at: u64) -> Option<u64> {
        (at != NEVER).then(|| self.now_ms().saturating_sub(at))
    }

    pub fn note_head(&self, head: u64) {
        self.source_head.store(head, Ordering::Release);
        self.source_head_at.store(self.now_ms(), Ordering::Release);
    }

    /// Record what the archive should hold by now. Sampled together with the head so the two
    /// never describe different moments.
    pub fn note_mature_target(&self, target: MatureTarget) {
        let raw = match target {
            MatureTarget::Unknown => NEVER,
            MatureTarget::NothingMature => NOTHING_MATURE,
            MatureTarget::Height(h) => h.min(NOTHING_MATURE - 1),
        };
        self.mature_target.store(raw, Ordering::Release);
    }

    pub fn note_progress(&self, height: u64) {
        self.last_progress_height.store(height, Ordering::Release);
        self.last_progress_at
            .store(self.now_ms(), Ordering::Release);
    }

    pub fn note_flush_ok(&self) {
        self.last_flush_ok_at
            .store(self.now_ms(), Ordering::Release);
        *self
            .last_flush_error
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = None;
    }

    pub fn note_flush_error(&self, err: impl ToString) {
        self.flush_errors.fetch_add(1, Ordering::AcqRel);
        *self
            .last_flush_error
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(err.to_string());
    }

    pub fn note_reconnect(&self) {
        self.reconnects.fetch_add(1, Ordering::AcqRel);
    }

    /// Evaluate readiness against the store's view. Pure apart from reading the clock.
    pub fn snapshot(
        &self,
        latest_archived_block: Option<u64>,
        total_blocks: usize,
    ) -> HealthSnapshot {
        let head_at = self.source_head_at.load(Ordering::Acquire);
        let source_head = (head_at != NEVER).then(|| self.source_head.load(Ordering::Acquire));
        let source_head_age_ms = self.age(head_at);
        let raw_target = self.mature_target.load(Ordering::Acquire);
        let target_known = raw_target != NEVER;
        let mature_target = Some(raw_target).filter(|t| *t != NEVER && *t != NOTHING_MATURE);
        let lag_blocks = match (mature_target, latest_archived_block) {
            (Some(target), Some(latest)) => Some(target.saturating_sub(latest)),
            (Some(target), None) => Some(target),
            _ => None,
        };
        let last_flush_error = self
            .last_flush_error
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();

        let mut not_ready_reasons = Vec::new();
        match source_head_age_ms {
            None => not_ready_reasons.push("source head never observed".to_owned()),
            Some(age) if age > self.stale_after.as_millis() as u64 => {
                not_ready_reasons.push(format!(
                    "source head sample is {age} ms old (stale after {} ms)",
                    self.stale_after.as_millis()
                ))
            }
            _ => {}
        }
        // Without any head sample the target is unknown for the same reason; say it once.
        if !target_known && source_head.is_some() {
            not_ready_reasons.push(
                "mature target not known yet (no head sample, or no attested height read)"
                    .to_owned(),
            );
        }
        match lag_blocks {
            Some(lag) if lag > self.ready_lag_blocks => not_ready_reasons.push(format!(
                "{lag} blocks behind the mature target (allowed {})",
                self.ready_lag_blocks
            )),
            _ => {}
        }
        if let Some(err) = &last_flush_error {
            not_ready_reasons.push(format!("last flush failed: {err}"));
        }

        HealthSnapshot {
            chain_id: self.chain_id,
            bound: self.bound.clone(),
            uptime_ms: self.now_ms(),
            latest_archived_block,
            total_blocks,
            source_head,
            source_head_age_ms,
            mature_target,
            lag_blocks,
            last_progress_age_ms: self.age(self.last_progress_at.load(Ordering::Acquire)),
            last_flush_ok_age_ms: self.age(self.last_flush_ok_at.load(Ordering::Acquire)),
            flush_errors: self.flush_errors.load(Ordering::Acquire),
            last_flush_error,
            reconnects: self.reconnects.load(Ordering::Acquire),
            ready: not_ready_reasons.is_empty(),
            not_ready_reasons,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn health() -> Health {
        Health::new(56, "test bound", 1_000, Duration::from_secs(60))
    }

    /// A head sample with a fixed lag of 10 behind it, the way the poller feeds both at once.
    fn observe(h: &Health, head: u64) {
        h.note_head(head);
        h.note_mature_target(MatureTarget::Height(head - 10));
    }

    #[test]
    fn not_ready_until_the_source_head_has_been_observed() {
        let h = health();
        let s = h.snapshot(Some(100), 101);
        assert!(!s.ready);
        assert_eq!(
            s.not_ready_reasons,
            vec!["source head never observed".to_owned()]
        );
        assert_eq!(s.lag_blocks, None);
    }

    #[test]
    fn ready_when_within_lag_and_head_is_fresh() {
        let h = health();
        observe(&h, 1_000_500);
        h.note_progress(1_000_000);
        let s = h.snapshot(Some(1_000_000), 1);
        assert_eq!(s.mature_target, Some(1_000_490));
        assert_eq!(s.lag_blocks, Some(490));
        assert!(s.ready, "{:?}", s.not_ready_reasons);
    }

    #[test]
    fn a_halted_source_with_full_coverage_is_still_ready() {
        // No progress for a long time is fine as long as we have observed the head recently
        // and we are caught up to it: the chain is idle, not the archiver.
        let h = health();
        observe(&h, 5_000);
        let s = h.snapshot(Some(4_990), 4_991);
        assert_eq!(s.lag_blocks, Some(0));
        assert!(s.ready);
    }

    #[test]
    fn falling_behind_the_mature_target_is_not_ready() {
        let h = health();
        observe(&h, 50_000);
        let s = h.snapshot(Some(10_000), 10_001);
        assert!(!s.ready);
        assert_eq!(s.lag_blocks, Some(39_990));
        assert!(s.not_ready_reasons[0].contains("blocks behind"));
    }

    #[test]
    fn flush_errors_block_readiness_until_the_next_successful_flush() {
        let h = health();
        observe(&h, 100);
        h.note_flush_error("disk full");
        let s = h.snapshot(Some(90), 91);
        assert!(!s.ready);
        assert_eq!(s.flush_errors, 1);
        assert_eq!(s.last_flush_error.as_deref(), Some("disk full"));
        h.note_flush_ok();
        let s = h.snapshot(Some(90), 91);
        assert!(s.ready);
        assert_eq!(
            s.flush_errors, 1,
            "the counter is history, the error is cleared"
        );
        assert_eq!(s.last_flush_error, None);
    }

    #[test]
    fn empty_archive_reports_the_whole_target_as_lag() {
        let h = Health::new(1, "test bound", 100, Duration::from_secs(60));
        h.note_head(250);
        h.note_mature_target(MatureTarget::Height(250));
        let s = h.snapshot(None, 0);
        assert_eq!(s.lag_blocks, Some(250));
        assert!(!s.ready);
    }

    #[test]
    fn a_known_empty_target_is_ready_with_unknown_lag() {
        // Nothing is mature yet (the chain is shorter than the fixed lag): the archiver holds
        // everything it should, and says so without inventing a lag.
        let h = health();
        h.note_head(5);
        h.note_mature_target(MatureTarget::NothingMature);
        let s = h.snapshot(None, 0);
        assert_eq!(s.mature_target, None);
        assert_eq!(s.lag_blocks, None);
        assert!(s.ready, "{:?}", s.not_ready_reasons);
    }

    #[test]
    fn the_target_is_what_was_fed_not_derived_from_the_head() {
        // Under an attested bound the target trails the head by whatever the attestors have
        // released, not by a fixed lag; a lagging target must not count as archive lag.
        let h = health();
        h.note_head(10_000);
        h.note_mature_target(MatureTarget::Height(8_000));
        let s = h.snapshot(Some(8_000), 8_001);
        assert_eq!(s.mature_target, Some(8_000));
        assert_eq!(s.lag_blocks, Some(0));
        assert!(s.ready);
    }

    #[test]
    fn an_unknown_target_is_not_ready_even_with_a_fresh_head() {
        // Right after start the attested height has not been read from Creditcoin yet. Treating
        // that as "nothing released" would report ready while the archive may be far behind an
        // attested height that is already published.
        let h = health();
        h.note_head(10_000);
        let s = h.snapshot(Some(10), 11);
        assert!(!s.ready);
        assert_eq!(s.mature_target, None);
        assert!(s.not_ready_reasons[0].contains("mature target not known"));
        h.note_mature_target(MatureTarget::Unknown);
        assert!(!h.snapshot(Some(10), 11).ready);
        h.note_mature_target(MatureTarget::Height(20));
        assert!(h.snapshot(Some(10), 11).ready);
    }
}
