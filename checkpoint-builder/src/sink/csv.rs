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
    writer: BufWriter<File>,
}

impl CsvFileSink {
    /// Create a new CSV file sink
    pub fn new(file_path: PathBuf) -> Self {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
            .with_context(|| format!("Failed to open CSV file: {}", file_path.display()))
            .expect("Failed to open CSV file for writing");

        let writer = BufWriter::new(file);

        Self { file_path, writer }
    }
}

impl CheckpointSink for CsvFileSink {
    fn write_checkpoints(&mut self, checkpoints: &[AttestationCheckpoint]) -> Result<()> {
        if checkpoints.is_empty() {
            return Ok(());
        }

        debug!(
            "Writing {} checkpoints to CSV file: {}",
            checkpoints.len(),
            self.file_path.display()
        );

        let mut line_buff = Vec::with_capacity(100); // block_number + comma + digest + newline

        // Build CSV records and write them
        for checkpoint in checkpoints {
            let block_number = checkpoint.block_number;
            let digest_hex = hex::encode(checkpoint.digest.as_bytes());

            writeln!(&mut line_buff, "{block_number},0x{digest_hex}")?;

            self.writer
                .write_all(&line_buff)
                .with_context(|| "Failed to write checkpoint to CSV file")?;
            line_buff.clear();
        }

        self.writer
            .flush()
            .with_context(|| "Failed to flush CSV file")?;

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
        output_file.with_file_name(format!("output_{timestamp}.csv"))
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
                if let Err(e) = csv_sink.write_checkpoints(&batch) {
                    error!("Failed to write CSV batch: {}", e);
                    return Err(e);
                }
                batch.clear();
            }
        }

        if !batch.is_empty() {
            debug!(
                "Writing final batch of {} checkpoints to CSV before shutdown",
                batch.len()
            );

            if let Err(e) = csv_sink.write_checkpoints(&batch) {
                error!("Failed to write final CSV batch: {}", e);
                return Err(e);
            }
        }

        Ok(())
    });

    handle
}
