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
//! reconnecting" as alive — but only while those reconnects are *failing* (a genuine outage).
//! Reconnects that keep *succeeding* while nothing progresses are not an outage; they are an
//! internal loop futilely redialing a healthy node (e.g. a backfill stuck on permanently
//! pruned state), which is exactly a wedge. Every reconnect path funnels through
//! `cc_client::Client::reconnect`, which timestamps each attempt and counts each success; we
//! read both here.
//!
//! Three independent axes decide liveness:
//!
//!   1. **cc3 progress** — a cc3 finalized batch was handled recently, or reconnects are being
//!      attempted *and failing* (outage ride-out). Catches a fully wedged production loop.
//!   2. **attestation production** — when this attestor is in the active set, it must produce
//!      local attestations while the network visibly finalizes them on cc3. "The chain attests
//!      but we haven't produced for a long time" means our source-chain (eth) pipeline stalled
//!      even though the cc3 side keeps turning — invisible to axis 1 by construction. This axis
//!      is relative (compares us to the network), so a network-wide halt (source chain down for
//!      everyone) does not trip it — correctly, since a restart would not help. It is armed only
//!      after `STALL_DEADLINE` of continuous eligibility, so boot, election warm-up, and
//!      chill→re-elect transitions get a generous grace window.
//!   3. **p2p connectivity** — when quorum requires peers and the network is visibly finalizing
//!      attestations, this process must not remain connectionless for a full liveness window.
//!      The network-progress condition prevents a network-wide p2p outage from causing every pod
//!      to restart together.
//!
//!   healthy  ⇔  not faulted
//!             ∧ not stalled (axis 2)
//!             ∧ not isolated (axis 3)
//!             ∧ ( recent cc3 progress
//!               ∨ recently attempted reconnects that are failing )
//!
//! Boot is covered by initializing every timestamp to construction time: the deadlines are
//! wider than any realistic boot window, so a fresh pod counts as healthy until real ticks land.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Max time without either forward progress or a reconnect attempt before we declare the binary
/// wedged. Must comfortably exceed both the slowest healthy cadence (cc3 finalized-block interval
/// plus any catch-up backfill) and the boot window (connect, genesis wait, initial catch-up), so
/// a quiet-but-healthy node — or a freshly started one — is never killed.
const LIVENESS_DEADLINE_MS: u64 = 5 * 60 * 1_000;

/// Max time an *eligible* attestor may go without producing a local attestation while the network
/// visibly finalizes attestations on cc3, before we declare the source-chain pipeline stalled.
/// Also the arming grace after eligibility begins. Deliberately much wider than
/// [`LIVENESS_DEADLINE_MS`]: local production leads on-chain finalization of the same height (we
/// are a signer), so a healthy node's local timestamp is never older than the chain timestamp by
/// more than aggregation + submission delay (minutes) — 3× the liveness deadline gives a large
/// margin over that and over any legitimate warm-up (election warm-up is 30 s).
const STALL_DEADLINE_MS: u64 = 3 * LIVENESS_DEADLINE_MS;

/// How many *successful* reconnects since the last observed progress it takes to stop treating
/// "actively reconnecting" as alive. Failing reconnects never count (a genuine outage stays
/// `Reconnecting` indefinitely); a couple of successes covers the blip-then-recover race where
/// the connection lands moments before the first batch does.
const RECONNECT_FUTILITY_THRESHOLD: u64 = 3;

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
    /// Unix-millis of the last `BlockAttested` observed on cc3 — the network (any attestor,
    /// possibly us) finalizing attestation progress on-chain. Seeded to construction time.
    last_chain_attested_ms: AtomicU64,
    /// Unix-millis of the last locally produced attestation (`emit_local`). Seeded to
    /// construction time so a fresh pod gets the full stall deadline before axis 2 can trip.
    last_local_attested_ms: AtomicU64,
    /// Whether this attestor currently believes it should be producing (mirrors `can_attest`).
    /// Axis 2 is inert while false — a chilled / not-elected attestor legitimately produces
    /// nothing.
    attest_expected: AtomicBool,
    /// Whether quorum requires another attestor. Single-attestor development committees do not
    /// need a p2p connection and must not trip the isolation axis.
    p2p_expected: AtomicBool,
    /// Distinct connected peers, maintained from first-connection/last-disconnection swarm
    /// events. Used only for liveness; Prometheus keeps its own gauge for observability.
    connected_peers: AtomicU64,
    /// Unix-millis when the peer count most recently fell to zero. Reset when a peer connects.
    peerless_since_ms: AtomicU64,
    /// Unix-millis of the most recent transition of `attest_expected`. Axis 2 arms only after
    /// `STALL_DEADLINE_MS` of continuous eligibility, so re-election after a long chill doesn't
    /// trip on the ancient local-production timestamp.
    attest_expected_since_ms: AtomicU64,
    /// `cc_client` reconnect-success count snapshotted at the last `Progressing` evaluation.
    /// The delta against the live count says how many reconnects *succeeded* since progress
    /// stalled — the futile-reconnect signal.
    reconnect_success_baseline: AtomicU64,
    /// Set once and never cleared: the supervisor detected a task fault/early-exit. The pod is on
    /// its way down; report unhealthy immediately so k8s doesn't route or wait on it.
    faulted: AtomicBool,
    /// False until the tasks spawn ([`Health::note_started`]). While false, everything except a
    /// latched fault reads `Starting`: the startup phases (election wait, BLS fetch) produce no
    /// progress ticks by nature, and legitimate transient blips during a days-long wait would
    /// otherwise accumulate toward the futility threshold.
    started: AtomicBool,
}

/// Outcome of a liveness check — `Display`s to a short body for the HTTP response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    /// Startup phases before the tasks spawn (endpoint waits, election wait — potentially days —
    /// BLS fetch). Alive: killing a pod for waiting on an election would just repeat the wait.
    /// Entered at construction, left via [`Health::note_started`] when the tasks spawn.
    Starting,
    Progressing,
    Reconnecting,
    Faulted,
    Wedged,
    /// The cc3 side is turning but this eligible attestor has produced nothing for
    /// [`STALL_DEADLINE_MS`] while the network keeps finalizing attestations — the source-chain
    /// pipeline is stalled. Restart-worthy: a fresh boot rebuilds the eth streams.
    Stalled,
    /// This eligible attestor requires peers, the network is still finalizing attestations, but
    /// this process has had no p2p connections for a full liveness window.
    Isolated,
}

impl Liveness {
    /// Whether k8s should consider the pod alive (200) vs. restart it (503).
    pub fn is_alive(self) -> bool {
        !matches!(
            self,
            Liveness::Faulted | Liveness::Wedged | Liveness::Stalled | Liveness::Isolated
        )
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
            Liveness::Stalled => "stalled",
            Liveness::Isolated => "isolated",
        };
        f.write_str(s)
    }
}

impl Health {
    pub fn new() -> Self {
        let now = now_unix_ms();
        Self {
            last_progress_ms: AtomicU64::new(now),
            last_chain_attested_ms: AtomicU64::new(now),
            last_local_attested_ms: AtomicU64::new(now),
            // Tasks spawn only after startup's `wait_for_eligible` + warm-up succeeded, so at
            // construction time this attestor is eligible. Production keeps it current on every
            // election / chill / kick event.
            attest_expected: AtomicBool::new(true),
            p2p_expected: AtomicBool::new(false),
            connected_peers: AtomicU64::new(0),
            peerless_since_ms: AtomicU64::new(now),
            attest_expected_since_ms: AtomicU64::new(now),
            reconnect_success_baseline: AtomicU64::new(0),
            faulted: AtomicBool::new(false),
            started: AtomicBool::new(false),
        }
    }

    /// Leave the `Starting` state — called when the tasks spawn. Re-seeds every timestamp so the
    /// deadlines measure from task start, not from construction: `Health` is created *before*
    /// the startup waits (so `/health` exists during them), which can be days for an election —
    /// stale construction-time seeds would otherwise read as an instant wedge/stall.
    pub fn note_started(&self) {
        let now = now_unix_ms();
        self.last_progress_ms.store(now, Ordering::Relaxed);
        self.last_chain_attested_ms.store(now, Ordering::Relaxed);
        self.last_local_attested_ms.store(now, Ordering::Relaxed);
        self.attest_expected_since_ms.store(now, Ordering::Relaxed);
        if self.connected_peers.load(Ordering::Relaxed) == 0 {
            self.peerless_since_ms.store(now, Ordering::Relaxed);
        }
        self.started.store(true, Ordering::Relaxed);
    }

    /// Record forward progress. Called from the production task whenever it handles a cc3
    /// finalized batch (the steady pulse) and on genesis finalization.
    pub fn note_progress(&self) {
        self.last_progress_ms
            .store(now_unix_ms(), Ordering::Relaxed);
    }

    /// Record that the network finalized an attestation on cc3 (`BlockAttested` observed —
    /// ours or a peer's). Axis 2's evidence that the source chain is attestable right now.
    pub fn note_chain_attested(&self) {
        self.last_chain_attested_ms
            .store(now_unix_ms(), Ordering::Relaxed);
    }

    /// Record a locally produced attestation (`emit_local`). Axis 2's evidence that our own
    /// source-chain pipeline is alive.
    pub fn note_local_attested(&self) {
        self.last_local_attested_ms
            .store(now_unix_ms(), Ordering::Relaxed);
    }

    /// Re-arm axis 2 after an attestation-chain reversion. A deep revert clears the proof cache
    /// and rolls the source stream back; rebuilding to the first post-revert `emit_local` can
    /// legitimately exceed `STALL_DEADLINE_MS` while *peers* keep finalizing (uneven recovery) —
    /// exactly axis 2's trigger shape. Stamping the local-attested clock grants recovery one
    /// full stall window before it can be flagged, instead of the pod being restarted (and
    /// re-starting the same rebuild) mid-catch-up.
    pub fn note_revert_recovery(&self) {
        self.last_local_attested_ms
            .store(now_unix_ms(), Ordering::Relaxed);
    }

    /// Mirror `can_attest` transitions. Stamps the transition time so axis 2 re-arms with a
    /// fresh grace window whenever eligibility begins (boot, re-election after a chill).
    pub fn set_attest_expected(&self, expected: bool) {
        let was = self.attest_expected.swap(expected, Ordering::Relaxed);
        if was != expected {
            let now = now_unix_ms();
            self.attest_expected_since_ms.store(now, Ordering::Relaxed);
            if expected && self.connected_peers.load(Ordering::Relaxed) == 0 {
                self.peerless_since_ms.store(now, Ordering::Relaxed);
            }
        }
    }

    pub fn set_p2p_expected(&self, expected: bool) {
        let was = self.p2p_expected.swap(expected, Ordering::Relaxed);
        if expected && !was && self.connected_peers.load(Ordering::Relaxed) == 0 {
            self.peerless_since_ms
                .store(now_unix_ms(), Ordering::Relaxed);
        }
    }

    pub fn note_peer_connected(&self) {
        if self.connected_peers.fetch_add(1, Ordering::Relaxed) == 0 {
            self.peerless_since_ms.store(0, Ordering::Relaxed);
        }
    }

    pub fn note_peer_disconnected(&self) {
        let previous = self
            .connected_peers
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                count.checked_sub(1)
            })
            .unwrap_or(0);
        if previous == 1 {
            self.peerless_since_ms
                .store(now_unix_ms(), Ordering::Relaxed);
        }
    }

    /// Latch the pod as faulted. Called by the supervisor when a task fails or exits early; the
    /// process is shutting down, so this never needs clearing.
    pub fn note_fault(&self) {
        self.faulted.store(true, Ordering::Relaxed);
    }

    /// Evaluate liveness. `cc3` supplies the reconnect attempt timestamp + success count so an
    /// outage that is actively (and unsuccessfully) being ridden out reads as alive rather than
    /// wedged. Those stay on `cc_client::Client` rather than being mirrored into `Health` so
    /// there's exactly one writer for them and no sync coupling — `Health` just observes.
    pub fn liveness(&self, cc3: &cc_client::Client) -> Liveness {
        self.observe(
            now_unix_ms(),
            cc3.last_reconnect_unix_ms(),
            cc3.reconnect_success_count(),
        )
    }

    /// Decision core. Reads its own timestamps; `now`, `last_reconnect_ms` and
    /// `reconnect_successes` are passed in so tests can drive every axis without wall-time or a
    /// cc3 client. Mutates one piece of state: while progressing, the reconnect-success baseline
    /// is re-snapshotted so the futility delta always measures "successes since progress stalled".
    fn observe(&self, now: u64, last_reconnect_ms: u64, reconnect_successes: u64) -> Liveness {
        if self.faulted.load(Ordering::Relaxed) {
            return Liveness::Faulted;
        }

        // Startup phases produce no progress ticks by design; every other axis is meaningless
        // until the tasks actually run (note_started re-seeds the timestamps at that point).
        if !self.started.load(Ordering::Relaxed) {
            return Liveness::Starting;
        }

        // Axis 2 — production stall, checked before the cc3-progress axis because its whole
        // point is that cc3 progress keeps flowing while the source-chain pipeline is dead.
        let expected = self.attest_expected.load(Ordering::Relaxed);
        let expected_since = self.attest_expected_since_ms.load(Ordering::Relaxed);
        let chain_attested = self.last_chain_attested_ms.load(Ordering::Relaxed);
        let local_attested = self.last_local_attested_ms.load(Ordering::Relaxed);
        let p2p_expected = self.p2p_expected.load(Ordering::Relaxed);
        let connected_peers = self.connected_peers.load(Ordering::Relaxed);
        let peerless_since = self.peerless_since_ms.load(Ordering::Relaxed);
        if expected
            && p2p_expected
            && connected_peers == 0
            && now.saturating_sub(peerless_since) >= LIVENESS_DEADLINE_MS
            && now.saturating_sub(chain_attested) < LIVENESS_DEADLINE_MS
        {
            return Liveness::Isolated;
        }
        if expected
            && now.saturating_sub(expected_since) >= STALL_DEADLINE_MS
            && now.saturating_sub(chain_attested) < LIVENESS_DEADLINE_MS
            && now.saturating_sub(local_attested) >= STALL_DEADLINE_MS
        {
            return Liveness::Stalled;
        }

        // Axis 1 — cc3 progress / outage ride-out.
        let last_progress_ms = self.last_progress_ms.load(Ordering::Relaxed);
        if now.saturating_sub(last_progress_ms) < LIVENESS_DEADLINE_MS {
            // Progressing: re-baseline the success counter so a later stall measures from here.
            self.reconnect_success_baseline
                .store(reconnect_successes, Ordering::Relaxed);
            return Liveness::Progressing;
        }
        if now.saturating_sub(last_reconnect_ms) < LIVENESS_DEADLINE_MS {
            let baseline = self.reconnect_success_baseline.load(Ordering::Relaxed);
            if reconnect_successes.saturating_sub(baseline) >= RECONNECT_FUTILITY_THRESHOLD {
                // Reconnects keep *succeeding* yet nothing progresses — not an outage, an
                // internal loop futilely redialing a healthy node. Restart-worthy.
                return Liveness::Wedged;
            }
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

    /// Build a started `Health` with every timestamp seeded to a known value so `observe` is
    /// exercised against a fixed clock instead of wall-time.
    fn health_at(ts: u64) -> Health {
        Health {
            last_progress_ms: AtomicU64::new(ts),
            last_chain_attested_ms: AtomicU64::new(ts),
            last_local_attested_ms: AtomicU64::new(ts),
            attest_expected: AtomicBool::new(true),
            p2p_expected: AtomicBool::new(false),
            connected_peers: AtomicU64::new(0),
            peerless_since_ms: AtomicU64::new(ts),
            attest_expected_since_ms: AtomicU64::new(ts),
            reconnect_success_baseline: AtomicU64::new(0),
            faulted: AtomicBool::new(false),
            started: AtomicBool::new(true),
        }
    }

    #[test]
    fn starting_until_note_started_regardless_of_staleness() {
        let h = Health::new();
        // Way past every deadline with no signals — still Starting (alive): the election wait
        // can legitimately take days and produces no ticks.
        let now = 100 * STALL_DEADLINE_MS;
        assert_eq!(h.observe(now, 0, 50), Liveness::Starting);
        // Fault still wins during startup.
        h.note_fault();
        assert_eq!(h.observe(now, 0, 0), Liveness::Faulted);
    }

    #[test]
    fn note_started_reseeds_deadlines() {
        let h = Health::new();
        h.note_started();
        // Immediately after starting, everything reads Progressing — the (very stale)
        // construction-era view must not carry over into the running state.
        assert_eq!(h.observe(now_unix_ms(), 0, 0), Liveness::Progressing);
    }

    #[test]
    fn faulted_overrides_everything() {
        let h = health_at(1_000);
        h.note_fault();
        // Even with fresh progress and a recent reconnect, a latched fault wins.
        assert_eq!(h.observe(1_000, 1_000, 0), Liveness::Faulted);
    }

    #[test]
    fn fresh_construction_reports_progressing() {
        // `Health::new()` seeds timestamps to construction time, so the boot window is
        // automatically counted as `Progressing` until either a real progress tick fires or the
        // liveness deadline elapses without one.
        let now = 12_345;
        let h = health_at(now);
        assert_eq!(h.observe(now, 0, 0), Liveness::Progressing);
    }

    #[test]
    fn recent_progress_is_progressing() {
        let now = LIVENESS_DEADLINE_MS;
        let h = health_at(now - (LIVENESS_DEADLINE_MS - 1));
        assert_eq!(h.observe(now, 0, 0), Liveness::Progressing);
    }

    #[test]
    fn stale_progress_but_recent_failing_reconnect_is_reconnecting() {
        let now = 10 * LIVENESS_DEADLINE_MS;
        // Progress is stale (older than the deadline) but a reconnect was attempted recently and
        // no successes have landed since the baseline — a genuine outage being ridden out.
        let h = health_at(now - (LIVENESS_DEADLINE_MS + 1));
        let last_reconnect = now - (LIVENESS_DEADLINE_MS - 1);
        assert_eq!(h.observe(now, last_reconnect, 0), Liveness::Reconnecting);
    }

    #[test]
    fn futile_succeeding_reconnects_are_wedged() {
        let start = 0;
        let h = health_at(start);
        // While progressing, successes are baselined away.
        assert_eq!(h.observe(start, 0, 5), Liveness::Progressing);
        // Progress goes stale; reconnects keep being attempted AND keep succeeding: once the
        // delta hits the futility threshold, reconnection is demonstrably not the problem.
        let now = start + LIVENESS_DEADLINE_MS + 1;
        // Keep axis 2 quiet for this test: chain attestation is as stale as progress.
        h.last_chain_attested_ms.store(start, Ordering::Relaxed);
        assert_eq!(h.observe(now, now, 5 + 2), Liveness::Reconnecting);
        assert_eq!(
            h.observe(now, now, 5 + RECONNECT_FUTILITY_THRESHOLD),
            Liveness::Wedged
        );
    }

    #[test]
    fn no_progress_and_no_reconnect_is_wedged() {
        let now = 10 * LIVENESS_DEADLINE_MS;
        // Both signals are stale past the deadline → genuine internal wedge.
        let stale = now - (LIVENESS_DEADLINE_MS + 1);
        let h = health_at(stale);
        assert_eq!(h.observe(now, stale, 0), Liveness::Wedged);
    }

    #[test]
    fn eth_stall_while_network_attests_is_stalled() {
        let start = 0;
        let h = health_at(start);
        let now = start + STALL_DEADLINE_MS + 1;
        // The cc3 side keeps turning (fresh progress + fresh chain attestations) but we have
        // produced nothing since `start` — past the stall deadline, with eligibility armed.
        h.last_progress_ms.store(now, Ordering::Relaxed);
        h.last_chain_attested_ms.store(now, Ordering::Relaxed);
        assert_eq!(h.observe(now, 0, 0), Liveness::Stalled);
    }

    #[test]
    fn prolonged_peerless_state_while_network_attests_is_isolated() {
        let start = 1;
        let h = health_at(start);
        h.p2p_expected.store(true, Ordering::Relaxed);
        let now = start + LIVENESS_DEADLINE_MS + 1;
        h.last_progress_ms.store(now, Ordering::Relaxed);
        h.last_chain_attested_ms.store(now, Ordering::Relaxed);
        h.last_local_attested_ms.store(now, Ordering::Relaxed);

        assert_eq!(h.observe(now, 0, 0), Liveness::Isolated);

        h.note_peer_connected();
        assert_eq!(h.observe(now, 0, 0), Liveness::Progressing);
    }

    #[test]
    fn stall_axis_inert_when_not_eligible() {
        let start = 0;
        let h = health_at(start);
        let now = start + STALL_DEADLINE_MS + 1;
        h.last_progress_ms.store(now, Ordering::Relaxed);
        h.last_chain_attested_ms.store(now, Ordering::Relaxed);
        // A chilled / not-elected attestor legitimately produces nothing.
        h.set_attest_expected(false);
        assert_eq!(h.observe(now, 0, 0), Liveness::Progressing);
    }

    #[test]
    fn stall_axis_rearms_with_grace_after_reelection() {
        let start = 0;
        let h = health_at(start);
        h.set_attest_expected(false);
        // Long chill, then re-elected shortly before `now`: the eligibility transition re-stamps
        // `attest_expected_since_ms`, so the ancient local timestamp cannot trip the axis until a
        // full stall deadline of continuous eligibility has elapsed.
        let reelected_at = 10 * STALL_DEADLINE_MS;
        let now = reelected_at + 1_000;
        h.attest_expected.store(true, Ordering::Relaxed);
        h.attest_expected_since_ms
            .store(reelected_at, Ordering::Relaxed);
        h.last_progress_ms.store(now, Ordering::Relaxed);
        h.last_chain_attested_ms.store(now, Ordering::Relaxed);
        assert_eq!(h.observe(now, 0, 0), Liveness::Progressing);
    }

    #[test]
    fn stall_axis_inert_when_whole_network_halted() {
        let start = 0;
        let h = health_at(start);
        let now = start + STALL_DEADLINE_MS + 1;
        // cc3 batches keep flowing but nobody (us included) attests: the source chain is down
        // for everyone — a restart would not help, so this must stay alive.
        h.last_progress_ms.store(now, Ordering::Relaxed);
        assert_eq!(h.observe(now, 0, 0), Liveness::Progressing);
    }

    #[test]
    fn local_production_keeps_stall_axis_quiet() {
        let start = 0;
        let h = health_at(start);
        let now = start + STALL_DEADLINE_MS + 1;
        h.last_progress_ms.store(now, Ordering::Relaxed);
        h.last_chain_attested_ms.store(now, Ordering::Relaxed);
        h.last_local_attested_ms
            .store(now - 1_000, Ordering::Relaxed);
        assert_eq!(h.observe(now, 0, 0), Liveness::Progressing);
    }

    #[test]
    fn liveness_alive_mapping() {
        assert!(Liveness::Starting.is_alive());
        assert!(Liveness::Progressing.is_alive());
        assert!(Liveness::Reconnecting.is_alive());
        assert!(!Liveness::Faulted.is_alive());
        assert!(!Liveness::Wedged.is_alive());
        assert!(!Liveness::Stalled.is_alive());
        assert!(!Liveness::Isolated.is_alive());
    }
}
