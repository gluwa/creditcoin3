use rand::{rng, Rng};
use std::time::Duration;
use tracing::{debug, info};

use super::{
    constants::{
        ATTESTATIONS_RESTART_WINDOW, BACKOFF_JITTER_DEN, BACKOFF_JITTER_NUM, BACKOFF_MAX,
        BACKOFF_MAX_ATTEMPTS, BACKOFF_MIN,
    },
    AttestorService, State,
};
use crate::error::Error;

/// Control-flow signal for the scheduler loop.
pub enum BackoffNext {
    Continue,
    Stop,
}

impl AttestorService {
    /// Called after each delay. Re-evaluates finality vs last vote head and either:
    /// - resumes (sets Running) and returns Stop, or
    /// - bumps attempt + reattach closer to finalized and returns Continue.
    pub async fn backoff_tick_once(&mut self) -> Result<BackoffNext, Error> {
        debug!("⏳ Backoff tick, checking finality vs last vote head");
        // You likely have logic like this already in note_last_attested_header / continuity checks.
        let Some((_last_finalized_digest, last_finalized_num)) =
            crate::engine::get_last_finalized(&self.cc3_client, self.chain_key()).await?
        else {
            // If nothing finalized at all, keep backing off up to the cap.
            self.bump_without_scheduling();
            return Ok(BackoffNext::Continue);
        };

        // Get the last voted block number from the voted_for map.
        // This is the last block we voted for, which we use to determine if we caught up.
        // We use next_back() to get the last element in the map, which is the most recent vote.
        let last_voted_for_block = self.voted_for.iter().next_back().map(|(n, _)| *n);

        // Decide if we caught up: within ATTESTATIONS_RESTART_WINDOW * interval of the last voted block
        let interval = self.cc3_client.get_attestation_interval();
        let caught_up = match last_voted_for_block {
            Some(v) => {
                let window = ATTESTATIONS_RESTART_WINDOW.saturating_mul(interval);
                last_finalized_num + window >= v
            }
            None => true, // nothing voted yet, safe to resume
        };

        match &mut self.state {
            State::PausedBackoff {
                attempt,
                total_paused,
                since,
                ..
            } => {
                let elapsed = since.elapsed();
                *total_paused += elapsed;

                if caught_up {
                    info!(
                        "🟢 Finality caught up within window, resuming (attempt={})",
                        *attempt
                    );
                    self.start_running().await?;
                    return Ok(BackoffNext::Stop);
                }

                debug!(
                    "🔴 Finality not caught up yet, last voted for block: {:?}, last finalized: {}, elapsed: {:?}, attempt: {}",
                    last_voted_for_block,
                    last_finalized_num,
                    elapsed,
                    *attempt
                );

                // Stop waiting for finality if we hit the max attempts.
                // This is a safety cap to avoid infinite backoff loops.
                if attempt == &BACKOFF_MAX_ATTEMPTS {
                    info!(
                        "🔴 Max backoff attempts reached ({}), stopping backoff loop",
                        BACKOFF_MAX_ATTEMPTS
                    );
                    // This will trigger a restart on next tick. See mod.rs for details.
                    return Ok(BackoffNext::Stop);
                }

                // Not caught up yet: increment attempt, reset since, optionally move subscription closer
                *attempt = attempt.saturating_add(1).min(BACKOFF_MAX_ATTEMPTS);
                *since = std::time::Instant::now();
                Ok(BackoffNext::Continue)
            }
            // If state changed under us, just stop the loop.
            _ => Ok(BackoffNext::Stop),
        }
    }

    /// Helper to bump counters when there is no finalized head yet.
    fn bump_without_scheduling(&mut self) {
        if let State::PausedBackoff {
            attempt,
            since,
            total_paused,
            ..
        } = &mut self.state
        {
            *total_paused += since.elapsed();
            *attempt = attempt.saturating_add(1).min(BACKOFF_MAX_ATTEMPTS);
            *since = std::time::Instant::now();
            debug!(
                "No finalized head yet, bumping backoff attempt to {}",
                *attempt
            );
        }
    }
}

pub fn jittered_backoff(attempt: u32) -> std::time::Duration {
    // Exponential backoff: min(2^attempt * BACKOFF_MIN, BACKOFF_MAX)
    let pow = attempt.min(BACKOFF_MAX_ATTEMPTS);
    let mut base = BACKOFF_MIN;
    for _ in 0..pow {
        base = base.saturating_mul(2);
        if base >= BACKOFF_MAX {
            base = BACKOFF_MAX;
            break;
        }
    }

    // Convert to ms safely
    let base_ms: u64 = u64::try_from(base.as_millis()).unwrap_or(u64::MAX);

    // Integer jitter span: base_ms * (NUM / DEN), computed in u128 to avoid overflow
    let span_u128 =
        (base_ms as u128).saturating_mul(BACKOFF_JITTER_NUM as u128) / (BACKOFF_JITTER_DEN as u128);

    // Clamp span to i64::MAX and materialize as i64 without lossy casts
    let span_i64: i64 = match i128::try_from(span_u128) {
        Ok(v_i128) => i64::try_from(v_i128).unwrap_or(i64::MAX),
        Err(_) => i64::MAX,
    };

    // Sample jitter in [-span, +span]
    let jitter_i64: i64 = rng().random_range(-span_i64..=span_i64);

    // Apply jitter; clamp to [0, u64::MAX] before building Duration
    let jittered_i128 = i128::from(base_ms) + i128::from(jitter_i64);
    let jittered_ms: u64 = if jittered_i128 <= 0 {
        0
    } else {
        u64::try_from(jittered_i128).unwrap_or(u64::MAX)
    };

    Duration::from_millis(jittered_ms)
}
