//! Liveness watchdog backing the `/health` endpoint.
//!
//! Purpose: let k8s restart a pod that is *wedged* — alive enough to keep serving `/metrics`,
//! but no longer making attestation progress (a deadlocked or live-locked task; the F2 zombie
//! that neither process-exit nor the supervisor's fail-fast catches, because a task that never
//! returns never reaches `join_next`).
//!
//! Design tension: a long upstream RPC outage *also* stops progress, but the attestor is
//! correctly riding it out (`retry.rs` / the cc3 stream reconnect unboundedly) — restarting the
//! pod would not fix upstream and would just crash-loop. So the watchdog treats "actively
//! reconnecting" as alive. Every reconnect path funnels through `cc_client::Client::reconnect`,
//! which timestamps each attempt; we read that timestamp here. Net effect:
//!
//!   healthy  ⇔  not faulted
//!             ∧ ( recent attestation progress
//!               ∨ recently attempted a reconnect )
//!
//! Only a genuine internal wedge — no progress *and* no reconnect effort, past the deadline —
//! reports unhealthy. Boot is covered by initializing `last_progress_ms` to construction time:
//! the liveness deadline (5 min) is wider than any realistic boot window (connect + genesis
//! wait + initial catch-up), so a fresh pod is automatically counted as "progressing" until its
//! first real tick lands.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Max time without either forward progress or a reconnect attempt before we declare the binary
/// wedged. Must comfortably exceed both the slowest healthy cadence (cc3 finalized-block interval
/// plus any catch-up backfill) and the boot window (connect, genesis wait, initial catch-up), so
/// a quiet-but-healthy node — or a freshly started one — is never killed.
const LIVENESS_DEADLINE_MS: u64 = 5 * 60 * 1_000;

/// Wall-clock unix-millis, saturating to 0 if the clock is before the epoch.
fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Shared liveness state. Held as `Arc<Health>` in [`crate::shared::Shared`].
#[derive(Debug)]
pub struct Health {
    /// Unix-millis of the last observed forward progress (a cc3 finalized batch handled).
    /// Seeded to construction time so the boot window is counted as healthy without needing a
    /// separate `started_at_ms` field — the liveness deadline alone covers the grace period.
    last_progress_ms: AtomicU64,
    /// Set once and never cleared: the supervisor detected a task fault/early-exit. The pod is on
    /// its way down; report unhealthy immediately so k8s doesn't route or wait on it.
    faulted: AtomicBool,
}

/// Outcome of a liveness check — `Display`s to a short body for the HTTP response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    Progressing,
    Reconnecting,
    Faulted,
    Wedged,
}

impl Liveness {
    /// Whether k8s should consider the pod alive (200) vs. restart it (503).
    pub fn is_alive(self) -> bool {
        !matches!(self, Liveness::Faulted | Liveness::Wedged)
    }
}

impl std::fmt::Display for Liveness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Liveness::Progressing => "progressing",
            Liveness::Reconnecting => "reconnecting",
            Liveness::Faulted => "faulted",
            Liveness::Wedged => "wedged",
        };
        f.write_str(s)
    }
}

impl Health {
    pub fn new() -> Self {
        Self {
            last_progress_ms: AtomicU64::new(now_unix_ms()),
            faulted: AtomicBool::new(false),
        }
    }

    /// Record forward progress. Called from the production task whenever it handles a cc3
    /// finalized batch (the steady pulse) and on genesis finalization.
    pub fn note_progress(&self) {
        self.last_progress_ms
            .store(now_unix_ms(), Ordering::Relaxed);
    }

    /// Latch the pod as faulted. Called by the supervisor when a task fails or exits early; the
    /// process is shutting down, so this never needs clearing.
    pub fn note_fault(&self) {
        self.faulted.store(true, Ordering::Relaxed);
    }

    /// Evaluate liveness. `cc3` supplies the last reconnect-attempt timestamp so an outage that
    /// is actively being ridden out reads as alive rather than wedged. The reconnect timestamp
    /// stays on `cc_client::Client` rather than being mirrored into `Health` so there's exactly
    /// one writer for it and no sync coupling — `Health` just observes.
    pub fn liveness(&self, cc3: &cc_client::Client) -> Liveness {
        self.decide(now_unix_ms(), cc3.last_reconnect_unix_ms())
    }

    /// Decision core. Reads `last_progress_ms` from `self`; `now` and `last_reconnect_ms` are
    /// passed in so tests can drive both axes without touching wall-time or the cc3 client.
    fn decide(&self, now: u64, last_reconnect_ms: u64) -> Liveness {
        if self.faulted.load(Ordering::Relaxed) {
            return Liveness::Faulted;
        }
        let last_progress_ms = self.last_progress_ms.load(Ordering::Relaxed);
        if now.saturating_sub(last_progress_ms) < LIVENESS_DEADLINE_MS {
            return Liveness::Progressing;
        }
        if now.saturating_sub(last_reconnect_ms) < LIVENESS_DEADLINE_MS {
            return Liveness::Reconnecting;
        }
        Liveness::Wedged
    }
}

impl Default for Health {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience: a shared handle.
pub type SharedHealth = Arc<Health>;

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `Health` with `last_progress_ms` seeded to a known value so `decide` is exercised
    /// against a fixed clock instead of wall-time.
    fn health_progressed_at(last_progress_ms: u64) -> Health {
        Health {
            last_progress_ms: AtomicU64::new(last_progress_ms),
            faulted: AtomicBool::new(false),
        }
    }

    #[test]
    fn faulted_overrides_everything() {
        let h = health_progressed_at(1_000);
        h.note_fault();
        // Even with fresh progress and a recent reconnect, a latched fault wins.
        assert_eq!(h.decide(1_000, 1_000), Liveness::Faulted);
    }

    #[test]
    fn fresh_construction_reports_progressing() {
        // `Health::new()` seeds `last_progress_ms` to construction time, so the boot window is
        // automatically counted as `Progressing` until either a real progress tick fires or the
        // liveness deadline elapses without one.
        let now = 12_345;
        let h = health_progressed_at(now);
        assert_eq!(h.decide(now, 0), Liveness::Progressing);
    }

    #[test]
    fn recent_progress_is_progressing() {
        let now = LIVENESS_DEADLINE_MS;
        let last_progress = now - (LIVENESS_DEADLINE_MS - 1);
        let h = health_progressed_at(last_progress);
        assert_eq!(h.decide(now, 0), Liveness::Progressing);
    }

    #[test]
    fn stale_progress_but_recent_reconnect_is_reconnecting() {
        let now = 10 * LIVENESS_DEADLINE_MS;
        // Progress is stale (older than the deadline) but a reconnect was attempted recently.
        let last_progress = now - (LIVENESS_DEADLINE_MS + 1);
        let last_reconnect = now - (LIVENESS_DEADLINE_MS - 1);
        let h = health_progressed_at(last_progress);
        assert_eq!(h.decide(now, last_reconnect), Liveness::Reconnecting);
    }

    #[test]
    fn no_progress_and_no_reconnect_is_wedged() {
        let now = 10 * LIVENESS_DEADLINE_MS;
        // Both signals are stale past the deadline → genuine internal wedge.
        let last_progress = now - (LIVENESS_DEADLINE_MS + 1);
        let last_reconnect = now - (LIVENESS_DEADLINE_MS + 1);
        let h = health_progressed_at(last_progress);
        assert_eq!(h.decide(now, last_reconnect), Liveness::Wedged);
    }

    #[test]
    fn liveness_alive_mapping() {
        assert!(Liveness::Progressing.is_alive());
        assert!(Liveness::Reconnecting.is_alive());
        assert!(!Liveness::Faulted.is_alive());
        assert!(!Liveness::Wedged.is_alive());
    }
}
