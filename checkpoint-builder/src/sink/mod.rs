use anyhow::Result;

use attestor_primitives::AttestationCheckpoint;

pub mod csv;

/// Generic trait for outputting generated checkpoints
pub trait CheckpointSink {
    /// Write a batch of checkpoints
    fn write_checkpoints(&mut self, checkpoints: Vec<AttestationCheckpoint>) -> Result<()>;
}
