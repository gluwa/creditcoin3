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
//!             ∧ ( within startup grace
//!               ∨ recent attestation progress
//!               ∨ recently attempted a reconnect )
//!
//! Only a genuine internal wedge — no progress *and* no reconnect effort, past startup — reports
//! unhealthy.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Max time without either forward progress or a reconnect attempt before we declare the binary
/// wedged. Must comfortably exceed the slowest healthy cadence (cc3 finalized-block interval plus
/// any catch-up backfill), so a quiet-but-healthy node is never killed.
const LIVENESS_DEADLINE_MS: u64 = 5 * 60 * 1_000;

/// Grace window after construction during which we always report alive, covering the boot path
/// (connect, genesis wait, initial catch-up) before the first progress tick lands. Pair with a
/// k8s `initialDelaySeconds` for belt-and-suspenders.
const STARTUP_GRACE_MS: u64 = 3 * 60 * 1_000;

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
    started_at_ms: u64,
    /// Unix-millis of the last observed forward progress (a cc3 finalized batch handled). 0 until
    /// the first tick.
    last_progress_ms: AtomicU64,
    /// Set once and never cleared: the supervisor detected a task fault/early-exit. The pod is on
    /// its way down; report unhealthy immediately so k8s doesn't route or wait on it.
    faulted: AtomicBool,
}

/// Outcome of a liveness check — `Display`s to a short body for the HTTP response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    Starting,
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
            Liveness::Starting => "starting",
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
            started_at_ms: now_unix_ms(),
            last_progress_ms: AtomicU64::new(0),
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
    /// is actively being ridden out reads as alive rather than wedged.
    pub fn liveness(&self, cc3: &cc_client::Client) -> Liveness {
        self.decide(
            now_unix_ms(),
            self.last_progress_ms.load(Ordering::Relaxed),
            cc3.last_reconnect_unix_ms(),
        )
    }

    /// Pure decision core (no clock / no I/O) so the policy is unit-testable.
    fn decide(&self, now: u64, last_progress_ms: u64, last_reconnect_ms: u64) -> Liveness {
        if self.faulted.load(Ordering::Relaxed) {
            return Liveness::Faulted;
        }
        if now.saturating_sub(self.started_at_ms) < STARTUP_GRACE_MS {
            return Liveness::Starting;
        }
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

    /// Build a `Health` with a fixed `started_at_ms` so `decide` is exercised against a known
    /// clock instead of wall-time.
    fn health_started_at(started_at_ms: u64) -> Health {
        Health {
            started_at_ms,
            last_progress_ms: AtomicU64::new(0),
            faulted: AtomicBool::new(false),
        }
    }

    #[test]
    fn faulted_overrides_everything() {
        let h = health_started_at(0);
        h.note_fault();
        // Even with fresh progress and a recent reconnect, a latched fault wins.
        assert_eq!(h.decide(1_000, 1_000, 1_000), Liveness::Faulted);
    }

    #[test]
    fn within_startup_grace_is_starting() {
        let h = health_started_at(0);
        // now - started_at < STARTUP_GRACE_MS, and no progress/reconnect yet.
        let now = STARTUP_GRACE_MS - 1;
        assert_eq!(h.decide(now, 0, 0), Liveness::Starting);
    }

    #[test]
    fn recent_progress_is_progressing() {
        let h = health_started_at(0);
        // Past startup grace, but progress is within the liveness deadline.
        let now = STARTUP_GRACE_MS + LIVENESS_DEADLINE_MS;
        let last_progress = now - (LIVENESS_DEADLINE_MS - 1);
        assert_eq!(h.decide(now, last_progress, 0), Liveness::Progressing);
    }

    #[test]
    fn stale_progress_but_recent_reconnect_is_reconnecting() {
        let h = health_started_at(0);
        let now = STARTUP_GRACE_MS + 10 * LIVENESS_DEADLINE_MS;
        // Progress is stale (older than the deadline) but a reconnect was attempted recently.
        let last_progress = now - (LIVENESS_DEADLINE_MS + 1);
        let last_reconnect = now - (LIVENESS_DEADLINE_MS - 1);
        assert_eq!(
            h.decide(now, last_progress, last_reconnect),
            Liveness::Reconnecting
        );
    }

    #[test]
    fn no_progress_and_no_reconnect_past_startup_is_wedged() {
        let h = health_started_at(0);
        let now = STARTUP_GRACE_MS + 10 * LIVENESS_DEADLINE_MS;
        // Both signals are stale past the deadline → genuine internal wedge.
        let last_progress = now - (LIVENESS_DEADLINE_MS + 1);
        let last_reconnect = now - (LIVENESS_DEADLINE_MS + 1);
        assert_eq!(
            h.decide(now, last_progress, last_reconnect),
            Liveness::Wedged
        );
    }

    #[test]
    fn liveness_alive_mapping() {
        assert!(Liveness::Starting.is_alive());
        assert!(Liveness::Progressing.is_alive());
        assert!(Liveness::Reconnecting.is_alive());
        assert!(!Liveness::Faulted.is_alive());
        assert!(!Liveness::Wedged.is_alive());
    }
}
