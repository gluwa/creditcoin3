use std::time::Instant;
use tracing::{debug, info};

/// Represents the state of the synchronization process.
///
/// This struct tracks the progress of syncing headers from an initial state
/// to a target state. It provides methods to update the state and log progress.
///
/// Fields:
/// - `initial_header`: The initial header number when syncing started.
/// - `last_finalized_attestation_header`: The last finalized attestation header number.
/// - `target_header`: The target header number to sync to.
/// - `last_update_time`: The last time the state was updated.
/// - `last_finalized`: The last finalized header number.
/// - `average_block_time_ms`: The smoothed average block time in milliseconds.
#[derive(Debug, Clone)]
pub struct SyncState {
    initial_header: u64,
    pub last_finalized_attested_header: u64,
    target_header: u64,
    last_update_time: Instant,
    last_finalized: u64,
    average_block_time_ms: u128, // EMA smoothed average
}

// Apply EMA smoothing: EMA_new = α * new + (1 - α) * old
const ALPHA_NUM: u128 = 1;
const ALPHA_DEN: u128 = 10; // α = 0.1

const REMAINING_BLOCKS_LOG_THRESHOLD: u128 = 40; // Don't log progress if remaining blocks are less than this

impl SyncState {
    /// Create a new `SyncState` at the start of syncing
    pub fn new(initial: u64, target: u64) -> Self {
        Self {
            initial_header: initial,
            last_finalized_attested_header: initial,
            target_header: target,
            last_update_time: Instant::now(),
            last_finalized: initial,
            average_block_time_ms: 1000, // default 1s per block until we get real timing
        }
    }

    pub fn current(&self) -> u64 {
        self.last_finalized_attested_header
    }

    /// Update the state with a new finalized header
    pub fn update(&mut self, new_finalized: u64, new_target: u64) {
        let now = Instant::now();

        let blocks_advanced = new_finalized.saturating_sub(self.last_finalized);
        let time_elapsed_ms = now.duration_since(self.last_update_time).as_millis().max(1); // prevent div-by-zero

        if blocks_advanced > 0 {
            let block_time_ms = time_elapsed_ms / u128::from(blocks_advanced);

            self.average_block_time_ms = (ALPHA_NUM * block_time_ms
                + (ALPHA_DEN - ALPHA_NUM) * self.average_block_time_ms)
                / ALPHA_DEN;

            self.last_finalized = new_finalized;
            self.last_update_time = now;
        }

        self.last_finalized_attested_header = new_finalized;
        self.target_header = new_target;
        self.log_progress();
    }

    /// Log current progress and ETA
    fn log_progress(&self) {
        let blocks_done = u128::from(self.last_finalized_attested_header - self.initial_header);
        let total_blocks = u128::from(self.target_header - self.initial_header);
        let blocks_remaining = u128::from(self.target_header - self.last_finalized_attested_header);

        if blocks_remaining <= REMAINING_BLOCKS_LOG_THRESHOLD {
            debug!("Sync is done, no need to log progress.");
            return;
        }

        if total_blocks == 0 {
            info!("Invalid sync range.");
            return;
        }

        let estimated_remaining_ms = blocks_remaining * self.average_block_time_ms;
        let total_secs = estimated_remaining_ms / 1000;

        let days = total_secs / 86_400;
        let hours = (total_secs % 86_400) / 3600;
        let minutes = (total_secs % 3600) / 60;
        let seconds = total_secs % 60;

        let percent = (blocks_done * 10_000) / total_blocks;
        let percent_whole = percent / 100;
        let percent_frac = percent % 100;

        info!(
            "⌛ Sync Progress: {}.{:02}% ({}/{}) | ETA: {}d {}h {}m {}s",
            percent_whole,
            percent_frac,
            self.last_finalized_attested_header,
            self.target_header,
            days,
            hours,
            minutes,
            seconds
        );
    }
}
