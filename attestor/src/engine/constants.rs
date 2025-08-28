/// Defines how many attestations to submit we keep in memory
pub const ATTESTATION_BUFFER_SIZE: usize = 100;

/// Defines how much finalized attestations can be used as a window to check if we already can restart the engine
pub const ATTESTATIONS_RESTART_WINDOW: u64 = 2;

// === Backoff tunables (testnet-friendly defaults) ===
pub const BACKOFF_MIN: std::time::Duration = std::time::Duration::from_secs(60);
// Half an hour max backoff, with a maximum of 7 attempts
pub const BACKOFF_MAX: std::time::Duration = std::time::Duration::from_secs(60 * 30);
pub const BACKOFF_MAX_ATTEMPTS: u32 = 7;
pub const BACKOFF_JITTER_NUM: u64 = 20;
pub const BACKOFF_JITTER_DEN: u64 = 100;
