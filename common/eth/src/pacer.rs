//! Client-side pacing of source-chain RPC requests.
//!
//! The block fetchers release whole ranges at once (startup catch-up, a `safe`/`finalized` tag
//! jumping an epoch, a stream reset), and the only bound on them is how many blocks are in flight.
//! That bounds concurrency, not request rate: against a fast provider ten blocks in flight is
//! hundreds of requests per second, far past what metered plans accept. A [`Pacer`] spaces
//! requests evenly so a burst drains at a steady rate instead.

use std::num::NonZeroU32;
use std::sync::Mutex;
use std::time::Duration;
use tokio::time::Instant;

/// Hands out request slots at most `rps` per second, shared by every clone of a
/// [`Client`](crate::Client).
///
/// Slots are evenly spaced with no burst allowance: the point is to smooth bursts out, not to
/// permit them. Without a rate the pacer never delays a request on its own, but
/// [`hold_off`](Self::hold_off) still applies, so a provider that answers "rate limited" pauses
/// every caller rather than only the one that hit it.
#[derive(Debug)]
pub struct Pacer {
    interval: Option<Duration>,
    state: Mutex<State>,
}

#[derive(Debug)]
struct State {
    /// Earliest time the next reservation may be served.
    next_slot: Instant,
    /// Bumped by every [`Pacer::hold_off`], so a caller that reserved its slot earlier can tell,
    /// when it wakes, that the slot is no longer valid.
    hold_offs: u64,
}

impl State {
    fn new() -> Mutex<Self> {
        Mutex::new(Self {
            next_slot: Instant::now(),
            hold_offs: 0,
        })
    }
}

impl Pacer {
    /// A pacer that only enforces [`hold_off`](Self::hold_off).
    pub fn unlimited() -> Self {
        Self {
            interval: None,
            state: State::new(),
        }
    }

    /// A pacer that admits at most `rps` requests per second.
    pub fn with_rate(rps: NonZeroU32) -> Self {
        Self {
            interval: Some(Duration::from_secs(1) / rps.get()),
            state: State::new(),
        }
    }

    /// Wait for the next request slot.
    ///
    /// Slots are reserved in call order, so concurrent callers are served first come, first
    /// served. A caller whose wait was overtaken by a [`hold_off`](Self::hold_off) reserves again
    /// behind it, so callers already queued are paused too and then leave at the paced rate
    /// rather than all at once. Dropping the future gives up the wait but not the reserved slot,
    /// which only delays later callers by one interval.
    pub async fn acquire(&self) {
        loop {
            let (slot, hold_offs) = {
                let mut state = self.lock();
                let slot = state.next_slot.max(Instant::now());
                state.next_slot = slot + self.interval.unwrap_or_default();
                (slot, state.hold_offs)
            };
            if slot > Instant::now() {
                tokio::time::sleep_until(slot).await;
            }
            if self.lock().hold_offs == hold_offs {
                return;
            }
        }
    }

    /// Hold every request back until at least `delay` from now. Used when a provider reports
    /// that we are rate limited: requests already sent still land, but nothing else goes out
    /// until the hold-off has passed, including callers already waiting in
    /// [`acquire`](Self::acquire).
    pub fn hold_off(&self, delay: Duration) {
        let mut state = self.lock();
        state.next_slot = state.next_slot.max(Instant::now() + delay);
        state.hold_offs += 1;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unlimited_does_not_wait() {
        let pacer = Pacer::unlimited();
        let start = Instant::now();
        for _ in 0..100 {
            pacer.acquire().await;
        }
        assert!(start.elapsed() < Duration::from_millis(50));
    }

    #[tokio::test]
    async fn rate_spaces_requests_evenly() {
        // 50 rps = one slot every 20 ms; the first slot is immediate, so 6 acquires span 100 ms.
        let pacer = Pacer::with_rate(NonZeroU32::new(50).unwrap());
        let start = Instant::now();
        for _ in 0..6 {
            pacer.acquire().await;
        }
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(100), "{elapsed:?}");
        assert!(elapsed < Duration::from_millis(300), "{elapsed:?}");
    }

    #[tokio::test]
    async fn concurrent_callers_share_the_rate() {
        let pacer = std::sync::Arc::new(Pacer::with_rate(NonZeroU32::new(50).unwrap()));
        let start = Instant::now();
        let tasks: Vec<_> = (0..6)
            .map(|_| {
                let pacer = pacer.clone();
                tokio::spawn(async move { pacer.acquire().await })
            })
            .collect();
        for task in tasks {
            task.await.unwrap();
        }
        assert!(start.elapsed() >= Duration::from_millis(100));
    }

    #[tokio::test]
    async fn hold_off_delays_even_an_unlimited_pacer() {
        let pacer = Pacer::unlimited();
        pacer.hold_off(Duration::from_millis(100));
        let start = Instant::now();
        pacer.acquire().await;
        assert!(start.elapsed() >= Duration::from_millis(90));
    }

    #[tokio::test]
    async fn hold_off_pauses_callers_already_waiting() {
        // 10 rps: the third caller's reserved slot is 200 ms out. A 500 ms hold-off issued once
        // the first two have gone must move it to after the hold-off, not let it go at 200 ms.
        let pacer = std::sync::Arc::new(Pacer::with_rate(NonZeroU32::new(10).unwrap()));
        let start = Instant::now();
        pacer.acquire().await;
        pacer.acquire().await;
        let queued = {
            let pacer = pacer.clone();
            tokio::spawn(async move {
                pacer.acquire().await;
                Instant::now()
            })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        pacer.hold_off(Duration::from_millis(500));
        let served = queued.await.unwrap();
        assert!(
            served - start >= Duration::from_millis(600),
            "{:?}",
            served - start
        );
    }

    #[tokio::test]
    async fn hold_off_never_shortens_an_existing_wait() {
        let pacer = Pacer::unlimited();
        pacer.hold_off(Duration::from_millis(100));
        pacer.hold_off(Duration::from_millis(10));
        let start = Instant::now();
        pacer.acquire().await;
        assert!(start.elapsed() >= Duration::from_millis(90));
    }
}
