//! Progress / freshness bookkeeping behind `/status` and `/ready`.
//!
//! Liveness (the process answers) and readiness (the archive is current and durable) are
//! deliberately separate. A halted source chain with a fresh head sample and full coverage is
//! healthy; a silently stale subscription with an unchanged height is not, even though both
//! return HTTP 200 on `/status`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;

/// Sentinel for "never happened" in the millisecond-offset atomics.
const NEVER: u64 = u64::MAX;

pub struct Health {
    started: Instant,
    /// `(chain_id, maturity)`, known only once the source-chain handshake completes.
    /// The API is up before that so probes can see the process; until then it is not ready.
    source: OnceLock<(u64, eth::Maturity)>,
    ready_lag_blocks: u64,
    stale_after: Duration,

    source_head: AtomicU64,
    source_head_at: AtomicU64,
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
    /// `null` until the source-chain handshake has completed.
    pub chain_id: Option<u64>,
    /// Only populated for a fixed-lag policy. Under a block tag the offset varies per head and
    /// is not knowable without an RPC call, so it is reported as `null` rather than guessed.
    pub finalization_lag: Option<u64>,
    pub uptime_ms: u64,
    pub latest_archived_block: Option<u64>,
    pub total_blocks: usize,
    pub source_head: Option<u64>,
    pub source_head_age_ms: Option<u64>,
    /// `source_head - finalization_lag`: the highest block the archiver should have by now.
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
    pub fn new(ready_lag_blocks: u64, stale_after: Duration) -> Self {
        Self {
            started: Instant::now(),
            source: OnceLock::new(),
            ready_lag_blocks,
            stale_after,
            source_head: AtomicU64::new(0),
            source_head_at: AtomicU64::new(NEVER),
            last_progress_height: AtomicU64::new(0),
            last_progress_at: AtomicU64::new(NEVER),
            last_flush_ok_at: AtomicU64::new(NEVER),
            flush_errors: AtomicU64::new(0),
            last_flush_error: Mutex::new(None),
            reconnects: AtomicU64::new(0),
        }
    }

    /// Record the verified source identity. Called once the ws/http/Creditcoin handshake and
    /// the chain-id pin have passed; a second call is ignored.
    pub fn set_source(&self, chain_id: u64, maturity: eth::Maturity) {
        let _ = self.source.set((chain_id, maturity));
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
        let source = self.source.get().copied();
        // Under a block tag the mature height is whatever the node reports, so it cannot be
        // derived from the head here. Report no target rather than a fabricated one; the lag
        // gate below then falls through and readiness rests on head freshness and flush health.
        // Phase 2 of the maturity rework replaces this with distance from the latest attestation,
        // which is knowable for both policies.
        let mature_target = match (source_head, source) {
            (Some(h), Some((_, eth::Maturity::FixedLag(lag)))) => Some(h.saturating_sub(lag)),
            _ => None,
        };
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
        if source.is_none() {
            not_ready_reasons.push("source chain handshake not complete".to_owned());
        }
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
            chain_id: source.map(|(id, _)| id),
            finalization_lag: source.and_then(|(_, maturity)| match maturity {
                eth::Maturity::FixedLag(lag) => Some(lag),
                eth::Maturity::Tag(_) => None,
            }),
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
        let h = Health::new(1_000, Duration::from_secs(60));
        h.set_source(56, eth::Maturity::FixedLag(10));
        h
    }

    #[test]
    fn not_ready_before_the_handshake_even_with_a_fresh_head() {
        let h = Health::new(1_000, Duration::from_secs(60));
        h.note_head(500);
        let s = h.snapshot(Some(490), 491);
        assert!(!s.ready);
        assert_eq!(s.chain_id, None);
        assert_eq!(s.mature_target, None, "no lag known, no target");
        assert_eq!(
            s.not_ready_reasons,
            vec!["source chain handshake not complete".to_owned()]
        );
        h.set_source(56, eth::Maturity::FixedLag(10));
        let s = h.snapshot(Some(490), 491);
        assert!(s.ready, "{:?}", s.not_ready_reasons);
        assert_eq!(s.chain_id, Some(56));
        // A second set_source is ignored.
        h.set_source(99, eth::Maturity::FixedLag(0));
        assert_eq!(h.snapshot(None, 0).chain_id, Some(56));
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
        h.note_head(1_000_500);
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
        h.note_head(5_000);
        let s = h.snapshot(Some(4_990), 4_991);
        assert_eq!(s.lag_blocks, Some(0));
        assert!(s.ready);
    }

    #[test]
    fn falling_behind_the_mature_target_is_not_ready() {
        let h = health();
        h.note_head(50_000);
        let s = h.snapshot(Some(10_000), 10_001);
        assert!(!s.ready);
        assert_eq!(s.lag_blocks, Some(39_990));
        assert!(s.not_ready_reasons[0].contains("blocks behind"));
    }

    #[test]
    fn flush_errors_block_readiness_until_the_next_successful_flush() {
        let h = health();
        h.note_head(100);
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

    /// A block-tag policy has no head-derived offset, so the target and the reported lag are
    /// both unknown. Readiness must then rest on head freshness and flush health rather than on
    /// a fabricated distance. Phase 2 of the maturity rework replaces this with distance from the
    /// latest attestation, which is knowable for both policies.
    #[test]
    fn a_tag_policy_reports_no_target_and_does_not_gate_on_lag() {
        let h = Health::new(1_000, Duration::from_secs(60));
        h.set_source(1, eth::Maturity::Tag(eth::BlockTag::Safe));
        h.note_head(1_000_000);
        let s = h.snapshot(Some(10), 11);
        assert_eq!(s.finalization_lag, None);
        assert_eq!(s.mature_target, None);
        assert_eq!(s.lag_blocks, None);
        assert!(
            s.ready,
            "a tag policy must not be gated on an unknowable lag: {:?}",
            s.not_ready_reasons
        );
    }

    #[test]
    fn empty_archive_reports_the_whole_target_as_lag() {
        let h = Health::new(100, Duration::from_secs(60));
        h.set_source(1, eth::Maturity::FixedLag(0));
        h.note_head(250);
        let s = h.snapshot(None, 0);
        assert_eq!(s.lag_blocks, Some(250));
        assert!(!s.ready);
    }
}
