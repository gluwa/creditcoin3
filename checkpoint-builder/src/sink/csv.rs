use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::Utc;
use tokio::sync::mpsc;
use tracing::{debug, error, info};

use attestor_primitives::AttestationCheckpoint;

use crate::sink::CheckpointSink;

/// CSV file sink for writing checkpoint data in CSV format
/// Writes data as: block_number,digest (no headers)
///
/// The writer is lazily initialized on first write and reused for subsequent writes.
pub struct CsvFileSink {
    file_path: PathBuf,
    writer: Option<BufWriter<File>>,
}

impl CsvFileSink {
    /// Create a new CSV file sink
    pub fn new(file_path: PathBuf) -> Self {
        Self {
            file_path,
            writer: None,
        }
    }
}

impl CheckpointSink for CsvFileSink {
    fn write_checkpoints(&mut self, checkpoints: Vec<AttestationCheckpoint>) -> Result<()> {
        if checkpoints.is_empty() {
            return Ok(());
        }

        debug!(
            "Writing {} checkpoints to CSV file: {}",
            checkpoints.len(),
            self.file_path.display()
        );

        // Lazily initialize the writer on first use
        if self.writer.is_none() {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.file_path)
                .with_context(|| {
                    format!("Failed to open CSV file: {}", self.file_path.display())
                })?;

            self.writer = Some(BufWriter::new(file));
            debug!("Opened CSV file for writing: {}", self.file_path.display());
        }

        let buff = self.writer.as_mut().expect("writer should be initialized");

        // Build CSV records and write them
        for checkpoint in &checkpoints {
            let formatted_digest = format!("0x{}", hex::encode(checkpoint.digest.as_bytes()));
            let line = format!("{},{}\n", checkpoint.block_number, formatted_digest);
            buff.write_all(line.as_bytes())
                .with_context(|| "Failed to write checkpoint to CSV file")?;
        }

        buff.flush().with_context(|| "Failed to flush CSV file")?;

        info!(
            "Successfully wrote {} checkpoints to CSV file: {}",
            checkpoints.len(),
            self.file_path.display()
        );

        Ok(())
    }
}

/// Spawns a background task that consumes checkpoints from a channel and writes them to CSV.
///
/// The task batches checkpoints according to `commit_interval` and writes them in bulk for efficiency.
/// The output filename will have a timestamp appended in the format YYYYMMDDTHHMMSS
/// (e.g., checkpoints_20260225T154718.csv)
pub async fn spawn_csv_sink(
    mut receiver: mpsc::Receiver<AttestationCheckpoint>,
    output_file: PathBuf,
    commit_interval: usize,
) -> tokio::task::JoinHandle<Result<()>> {
    // Add timestamp to output filename
    let timestamp = Utc::now().format("%Y%m%dT%H%M%S");
    let timestamped_file = if let Some(stem) = output_file.file_stem() {
        let extension = output_file
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let new_name = format!("{}_{}{}", stem.to_string_lossy(), timestamp, extension);
        output_file.with_file_name(new_name)
    } else {
        output_file.with_file_name(format!("output_{timestamp}"))
    };

    info!(
        "Spawning CSV sink task with output file: {} and commit interval: {}",
        timestamped_file.display(),
        commit_interval
    );

    let handle = tokio::spawn(async move {
        let mut csv_sink = CsvFileSink::new(timestamped_file);

        let mut batch = Vec::with_capacity(commit_interval);

        info!("CSV sink task started, waiting for checkpoints to write to file...");

        while let Some(checkpoint) = receiver.recv().await {
            debug!(
                "Received checkpoint for block {} with digest {}",
                checkpoint.block_number, checkpoint.digest
            );

            batch.push(checkpoint);

            // Write batch if we've reached the commit interval
            if batch.len() >= commit_interval {
                let batch_to_write = std::mem::take(&mut batch);

                if let Err(e) = csv_sink.write_checkpoints(batch_to_write) {
                    error!("Failed to write CSV batch: {}", e);
                    return Err(e);
                }
            }
        }

        if !batch.is_empty() {
            debug!(
                "Writing final batch of {} checkpoints to CSV before shutdown",
                batch.len()
            );

            if let Err(e) = csv_sink.write_checkpoints(batch) {
                error!("Failed to write final CSV batch: {}", e);
                return Err(e);
            }
        }

        Ok(())
    });

    handle
}
