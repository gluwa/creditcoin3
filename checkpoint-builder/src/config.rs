//! Configuration for the checkpoint generator tool.
//!
//! Supports both command-line arguments and environment variables via .env files.

use std::num::NonZeroUsize;
use std::path::PathBuf;

use anyhow::{Context, Result};
use attestor_primitives::Digest;
use clap::{Args, Parser, Subcommand};

/// Information about a single checkpoint range, including start/end heights and checkpoint interval.
///
/// There are two types of checkpoint ranges:
///
/// **Genesis Range:**
/// A single checkpoint at a specified block height. This is used to initialize the checkpoint chain
/// when starting from a known genesis block. The tool reads the block from the database and creates
/// the first checkpoint from it, enabling proper chaining with subsequent regular ranges.
/// Only the first range can be a genesis range.
///
/// **Regular Range:**
/// A contiguous range of blocks [height_start, height_end] (inclusive) processed with a specified
/// checkpoint_interval. Generates one checkpoint for every `checkpoint_interval` blocks.
/// The block count must be divisible by the interval.
///
/// **Example:**
/// - `Genesis(100)`: Creates a checkpoint at block 100
/// - `Regular(CheckpointRange { height_start: 100, height_end: 999, checkpoint_interval: 100 })`:
///   Creates 9 checkpoints (at blocks 200, 300, ..., 1000)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointRangeType {
    /// Genesis checkpoint at a single block height. Only the first range can be genesis.
    Genesis(u64),
    /// Regular range with multiple checkpoints at specified intervals.
    Regular(CheckpointRange),
}

impl CheckpointRangeType {
    /// Returns the starting block height for this range
    pub fn height_start(&self) -> u64 {
        match self {
            CheckpointRangeType::Genesis(height) => *height,
            CheckpointRangeType::Regular(range) => range.height_start,
        }
    }

    /// Returns the ending block height for this range
    pub fn height_end(&self) -> u64 {
        match self {
            CheckpointRangeType::Genesis(height) => *height,
            CheckpointRangeType::Regular(range) => range.height_end,
        }
    }
}

/// A regular (non-genesis) checkpoint range with specific interval.
///
/// Represents a contiguous range of blocks [height_start, height_end] (inclusive)
/// that will be processed with the specified checkpoint_interval.
///
/// **Validation Requirements:**
/// - `height_start <= height_end`
/// - `checkpoint_interval > 0`
/// - `(height_end - height_start + 1) % checkpoint_interval == 0` (block count evenly divisible)
/// - When multiple ranges exist, they must be contiguous: `prev.height_end + 1 == curr.height_start`
///
/// **Example:**
/// A range from block 1000 to 4999 with interval 500:
/// - Creates checkpoints at blocks: 500, 1000, 1500, 2000, 2500, 3000, 3500, 4000, 4500, 5000 (9 checkpoints)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointRange {
    /// First block height in this range (inclusive)
    pub height_start: u64,
    /// Last block height in this range (inclusive)
    pub height_end: u64,
    /// Number of blocks per checkpoint within this range
    pub checkpoint_interval: usize,
}

impl CheckpointRange {
    /// Returns the number of blocks in this range
    pub fn block_count(&self) -> u64 {
        self.height_end - self.height_start + 1
    }
}

/// Parse a single range from "genesis_height" or "start,end,interval" format.
///
/// Supports two formats:
/// - `"100"` → Genesis checkpoint at block 100
/// - `"1000,4999,500"` → Regular range from block 1000 to 4999 with interval 500
///
/// Only the first range (range_index == 0) can be a genesis checkpoint.
fn parse_single_range(s: &str, range_index: usize) -> Result<CheckpointRangeType> {
    let parts: Vec<&str> = s.split(',').collect();

    match parts.len() {
        1 => {
            // Genesis checkpoint case: "0" or "1000"
            let height: u64 = parts[0].trim().parse().with_context(|| {
                format!(
                    "Range {}: invalid genesis height '{}' - must be a non-negative integer",
                    range_index + 1,
                    parts[0]
                )
            })?;

            Ok(CheckpointRangeType::Genesis(height))
        }
        3 => {
            let height_start: u64 = parts[0].trim().parse().with_context(|| {
                format!(
                    "Range {}: invalid height_start '{}' - must be a non-negative integer",
                    range_index + 1,
                    parts[0]
                )
            })?;

            let height_end: u64 = parts[1].trim().parse().with_context(|| {
                format!(
                    "Range {}: invalid height_end '{}' - must be a non-negative integer",
                    range_index + 1,
                    parts[1]
                )
            })?;

            let checkpoint_interval: usize = parts[2].trim().parse().with_context(|| {
                format!(
                    "Range {}: invalid checkpoint_interval '{}' - must be a positive integer",
                    range_index + 1,
                    parts[2]
                )
            })?;

            Ok(CheckpointRangeType::Regular(CheckpointRange {
                height_start,
                height_end,
                checkpoint_interval,
            }))
        }
        _ => {
            anyhow::bail!(
                "Range {} has invalid format '{}': expected 'start,end,interval' or 'genesis_height'",
                range_index + 1,
                s
            );
        }
    }
}

/// Parse CHECKPOINT_RANGES from semicolon-separated format.
///
/// **Format:** `"range1;range2;..."` where each range is either:
/// - Genesis: `"height"` (single integer, only first range)
/// - Regular: `"start,end,interval"` (three integers)
///
/// **Examples:**
/// - `"0,999,100;1000,4999,500"` - Two regular ranges starting from block 0
/// - `"0;1,999,100;1000,4999,500"` - Genesis at block 0, then regular ranges
/// - `"100;101,999,100"` - Genesis at block 100, then regular range
///
/// **Parsing Behavior:**
/// - Empty input raises an error
/// - Each semicolon-separated part is parsed as either genesis or regular range
/// - Only the first range can be a genesis checkpoint (validated by validate_checkpoint_ranges)
pub fn parse_checkpoint_ranges(input: &str) -> Result<Vec<CheckpointRangeType>> {
    if input.trim().is_empty() {
        anyhow::bail!("CHECKPOINT_RANGES cannot be empty");
    }

    let ranges: Result<Vec<CheckpointRangeType>> = input
        .split(';')
        .enumerate()
        .map(|(i, s)| parse_single_range(s.trim(), i))
        .collect();

    ranges
}

/// Validate a vector of checkpoint ranges.
///
/// Ensures ranges are properly formatted and compatible with the checkpoint generation algorithm.
///
/// **Validation Rules:**
/// 1. **Non-empty:** At least one range must be provided
/// 2. **Genesis uniqueness:** Only the first range can be a genesis checkpoint
/// 3. **Range bounds:** Each regular range must have `height_start <= height_end`
/// 4. **Positive interval:** Each regular range must have `checkpoint_interval > 0`
/// 5. **Divisibility:** Each range's block count `(height_end - height_start + 1)` must be
///    evenly divisible by its `checkpoint_interval`. This ensures we get exact checkpoints
///    at range boundaries without partial chunks.
/// 6. **Ordering:** Ranges must be sorted by start height (ascending)
/// 7. **Contiguity:** Regular ranges must be contiguous with no gaps:
///    `range[i].height_end + 1 == range[i+1].height_start`
///    This ensures the checkpoint chain is complete across all ranges.
/// 8. **Genesis placement:** When present, genesis checkpoint must be before the first
///    regular range (implied by rule 7)
///
/// **Why validation matters:**
/// - Range contiguity ensures we can chain checkpoints across range boundaries
/// - Divisibility ensures no partial chunks that would violate checkpoint_interval expectations
/// - These constraints prevent silent data corruption or missing checkpoints
pub fn validate_checkpoint_ranges(ranges: &[CheckpointRangeType]) -> Result<()> {
    if ranges.is_empty() {
        anyhow::bail!("At least one checkpoint range is required");
    }

    // Validate each range individually
    for (i, range_type) in ranges.iter().enumerate() {
        if i > 0 && matches!(range_type, CheckpointRangeType::Genesis(_)) {
            anyhow::bail!(
                "Only the first range can be a genesis checkpoint, but range {} is a genesis checkpoint",
                i
            );
        }

        if let CheckpointRangeType::Regular(range) = range_type {
            if range.height_start > range.height_end {
                anyhow::bail!(
                    "Range {}: height_start ({}) must be <= height_end ({})",
                    i,
                    range.height_start,
                    range.height_end
                );
            }

            if range.checkpoint_interval == 0 {
                anyhow::bail!("Range {}: checkpoint_interval must be greater than 0", i);
            }

            let block_count = range.block_count();
            if block_count as usize % range.checkpoint_interval != 0 {
                anyhow::bail!(
                    "Range {}: block count ({}) must be divisible by checkpoint_interval \
                     ({}). Range covers blocks {} to {} (inclusive).",
                    i,
                    block_count,
                    range.checkpoint_interval,
                    range.height_start,
                    range.height_end
                );
            }
        }
    }

    // Validate cross-range constraints (ordering and contiguity)
    for i in 1..ranges.len() {
        match (&ranges[i - 1], &ranges[i]) {
            (CheckpointRangeType::Genesis(genesis_height), CheckpointRangeType::Regular(range)) => {
                if genesis_height + 1 != range.height_start {
                    anyhow::bail!(
                        "Range {}: genesis checkpoint at height {} \
                        must be before the first regular range starting at height {}",
                        i - 1,
                        genesis_height,
                        range.height_start
                    );
                }
            }
            (CheckpointRangeType::Regular(prev), CheckpointRangeType::Regular(curr)) => {
                // Check ordering
                if curr.height_start <= prev.height_end {
                    anyhow::bail!(
                        "Ranges must be ordered and non-overlapping: \
                        range {} ends at {} but range {} starts at {}",
                        i,
                        prev.height_end,
                        i + 1,
                        curr.height_start
                    );
                }

                // Check contiguity (prev.height_end + 1 == curr.height_start)
                if prev.height_end + 1 != curr.height_start {
                    anyhow::bail!(
                        "Ranges must be contiguous: range {} \
                            ends at {} but range {} starts at {} (expected {})",
                        i,
                        prev.height_end,
                        i + 1,
                        curr.height_start,
                        prev.height_end + 1
                    );
                }
            }
            (prev, curr) => anyhow::bail!(
                "Invalid range types: range {} is {:?} but range {} is {:?}. \
                    Only the first range can be a genesis checkpoint.",
                i,
                prev,
                i + 1,
                curr
            ),
        }
    }

    Ok(())
}

/// CLI configuration for the checkpoint generator
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: SourceCommand,
}

#[derive(Subcommand, Debug)]
pub enum SourceCommand {
    /// Read block roots from a local Sled database
    Sled(SledArgs),
    /// Read block roots from an archiver HTTP API
    Archiver(ArchiverArgs),
}

#[derive(Args, Debug)]
pub struct SledArgs {
    /// Path to the Sled database containing block roots
    #[arg(long, env = "SLED_DB_PATH")]
    pub sled_db_path: PathBuf,

    #[command(flatten)]
    pub common: CommonArgs,
}

#[derive(Args, Debug)]
pub struct ArchiverArgs {
    /// Base URL of the archiver HTTP API (e.g. http://localhost:8080)
    #[arg(long, env = "ARCHIVER_URL")]
    pub archiver_url: url::Url,

    #[command(flatten)]
    pub common: CommonArgs,
}

#[derive(Args, Debug, Clone)]
pub struct CommonArgs {
    /// Genesis checkpoint digest as a hex string (0x-prefixed, 32 bytes).
    ///
    /// This is the checkpoint digest at block (first_range_start - 1), used to initialize
    /// the checkpoint chain when resuming from a previous state.
    ///
    /// **When to provide:**
    /// - Required if the first range is a regular range (not a genesis checkpoint)
    /// - Must NOT be provided if the first range is a genesis checkpoint
    /// - Not needed when starting from block 0 with a genesis checkpoint
    ///
    /// **Example:** When resuming from a previous run that ended at block 4999,
    /// provide the checkpoint digest at block 4999 and start the first range at block 5000.
    #[arg(long, env = "STARTING_DIGEST")]
    pub starting_digest: Option<String>,

    /// Checkpoint ranges in format "start,end,interval;start,end,interval;..."
    /// Example: "0,999,100;1000,4999,500" creates checkpoints every 100 blocks
    /// for blocks 0-999, then every 500 blocks for blocks 1000-4999.
    #[arg(long, env = "CHECKPOINT_RANGES", required_unless_present = "dry_run")]
    pub checkpoint_ranges: Option<String>,

    /// When set, prints the available block range in the source and exits without generating checkpoints.
    #[arg(long, env = "DRY_RUN", default_value_t = false)]
    pub dry_run: bool,

    /// Number of checkpoints to batch together before committing to output file
    #[arg(long, env = "CHECKPOINT_FLUSH_INTERVAL", default_value_t = NonZeroUsize::new(20).unwrap())]
    pub checkpoint_flush_interval: NonZeroUsize,

    /// Checks whether the provided checkpoint ranges are all present in the source before starting processing.
    #[arg(long, env = "VALIDATE_DATABASE", default_value_t = false)]
    pub validate_database: bool,

    /// Output file path for generated checkpoints (CSV format)
    #[arg(long, env = "OUTPUT_FILE", default_value = "checkpoints.csv")]
    pub output_file: PathBuf,
}

/// Internal configuration structure combining CLI args and defaults
#[derive(Debug, Clone)]
pub struct CheckpointConfig {
    pub starting_digest: Option<Digest>,
    /// The checkpoint ranges to process
    pub ranges: Vec<CheckpointRangeType>,
    pub checkpoint_commit_interval: NonZeroUsize,
    pub validate_database: bool,
    pub output_file: PathBuf,
    pub dry_run: bool,
}

impl CheckpointConfig {
    /// Returns the starting block (first range's height_start)
    pub fn starting_block(&self) -> u64 {
        self.ranges
            .first()
            .map(|r| r.height_start())
            .expect("No checkpoint ranges defined")
    }

    /// Returns the ending block (last range's height_end)
    pub fn end_block(&self) -> u64 {
        self.ranges
            .last()
            .map(|r| r.height_end())
            .expect("No checkpoint ranges defined")
    }

    pub fn validate_ranges(&self, source: &dyn crate::source::RootSource) -> bool {
        let mut error_count = 0;

        for range in self.ranges.iter() {
            tracing::info!(
                "Validating range: {} to {}",
                range.height_start(),
                range.height_end()
            );

            match range {
                CheckpointRangeType::Genesis(height) => {
                    tracing::info!("Checking genesis checkpoint at height {}", height);

                    let v = match source.get(*height) {
                        Ok(maybe_root) => maybe_root,
                        Err(e) => {
                            tracing::error!(
                                "Error fetching genesis checkpoint at height {}: {}",
                                height,
                                e
                            );
                            return false;
                        }
                    };

                    if v.is_none() {
                        tracing::error!("Missing block root for height {} in source", height);
                        error_count += 1;
                    }
                }
                CheckpointRangeType::Regular(r) => {
                    tracing::info!(
                        "Checking regular range from {} to {} with interval {}",
                        r.height_start,
                        r.height_end,
                        r.checkpoint_interval
                    );

                    let v = match source.get_range(r.height_start, r.height_end) {
                        Ok(roots) => roots,
                        Err(e) => {
                            tracing::error!(
                                "Error fetching block roots for range {} to {}: {}",
                                r.height_start,
                                r.height_end,
                                e
                            );
                            return false;
                        }
                    };

                    if v.len() != r.block_count() as usize {
                        tracing::error!(
                            "Missing block root for range {} to {} in source",
                            r.height_start,
                            r.height_end
                        );
                        error_count += 1;
                    }
                }
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(500));

        error_count == 0
    }

    /// Create configuration from common CLI arguments (source-agnostic).
    pub fn from_common(args: CommonArgs) -> Result<Self> {
        // Parse and validate checkpoint ranges
        let ranges = match args.checkpoint_ranges {
            Some(ref ranges_str) => {
                let ranges = parse_checkpoint_ranges(ranges_str)?;
                validate_checkpoint_ranges(&ranges)?;
                ranges
            }
            None if args.dry_run => Vec::new(),
            None => anyhow::bail!("checkpoint_ranges is required when not in dry-run mode"),
        };

        let starting_block = ranges.first().map(|r| r.height_start()).unwrap_or(0);

        let starting_digest = match &args.starting_digest {
            Some(digest) if !digest.is_empty() => {
                if digest.len() != 66 || !digest.starts_with("0x") {
                    anyhow::bail!("Starting digest must be a 32-byte hex string prefixed with 0x");
                }

                if starting_block == 0 {
                    anyhow::bail!(
                        "Starting digest should not be provided when starting from block 0"
                    );
                }

                if matches!(ranges.first(), Some(CheckpointRangeType::Genesis(_))) {
                    anyhow::bail!(
                        "Starting digest should not be provided when the first range is a genesis checkpoint"
                    );
                }

                Some(Digest::from_slice(
                    &hex::decode(&digest[2..])
                        .with_context(|| "Failed to decode starting digest from hex")?,
                ))
            }
            _ => {
                if matches!(ranges.first(), Some(CheckpointRangeType::Regular(_))) {
                    anyhow::bail!(
                        "Starting digest is required when the first range is not a genesis range"
                    );
                }

                None
            }
        };

        Ok(Self {
            starting_digest,
            ranges,
            checkpoint_commit_interval: args.checkpoint_flush_interval,
            validate_database: args.validate_database,
            output_file: args.output_file,
            dry_run: args.dry_run,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_single_range() {
        let range = parse_single_range("0,999,100", 0).unwrap();

        if let CheckpointRangeType::Regular(range) = range {
            assert_eq!(range.height_start, 0);
            assert_eq!(range.height_end, 999);
            assert_eq!(range.checkpoint_interval, 100);
        } else {
            panic!("Expected regular range, got genesis");
        }
    }

    #[test]
    fn test_parse_checkpoint_ranges() {
        let ranges = parse_checkpoint_ranges("0,999,100;1000,4999,500").unwrap();

        if let CheckpointRangeType::Regular(range1) = &ranges[0] {
            assert_eq!(range1.height_start, 0);
            assert_eq!(range1.height_end, 999);
            assert_eq!(range1.checkpoint_interval, 100);
        } else {
            panic!("Expected first range to be regular");
        }

        if let CheckpointRangeType::Regular(range2) = &ranges[1] {
            assert_eq!(range2.height_start, 1000);
            assert_eq!(range2.height_end, 4999);
            assert_eq!(range2.checkpoint_interval, 500);
        } else {
            panic!("Expected second range to be regular");
        }
    }

    #[test]
    fn test_parse_checkpoint_ranges_empty() {
        assert!(parse_checkpoint_ranges("").is_err());
        assert!(parse_checkpoint_ranges("   ").is_err());
    }

    #[test]
    fn test_parse_checkpoint_ranges_invalid_format() {
        assert!(parse_checkpoint_ranges("0,999").is_err()); // missing interval
        assert!(parse_checkpoint_ranges("abc,999,100").is_err()); // invalid start
    }

    #[test]
    fn test_validate_ranges_valid() {
        let ranges = vec![
            CheckpointRangeType::Regular(CheckpointRange {
                height_start: 1,
                height_end: 1000,
                checkpoint_interval: 100,
            }),
            CheckpointRangeType::Regular(CheckpointRange {
                height_start: 1001,
                height_end: 5000,
                checkpoint_interval: 500,
            }),
        ];
        assert!(validate_checkpoint_ranges(&ranges).is_ok());
    }

    #[test]
    fn test_validate_ranges_empty() {
        assert!(validate_checkpoint_ranges(&[]).is_err());
    }

    #[test]
    fn test_validate_ranges_start_greater_than_end() {
        let ranges = vec![CheckpointRangeType::Regular(CheckpointRange {
            height_start: 1000,
            height_end: 500,
            checkpoint_interval: 100,
        })];
        assert!(validate_checkpoint_ranges(&ranges).is_err());
    }

    #[test]
    fn test_validate_ranges_zero_interval() {
        let ranges = vec![CheckpointRangeType::Regular(CheckpointRange {
            height_start: 0,
            height_end: 999,
            checkpoint_interval: 0,
        })];
        assert!(validate_checkpoint_ranges(&ranges).is_err());
    }

    #[test]
    fn test_validate_ranges_not_divisible() {
        let ranges = vec![CheckpointRangeType::Regular(CheckpointRange {
            height_start: 0,
            height_end: 999,
            checkpoint_interval: 300, // 1000 blocks not divisible by 300
        })];
        assert!(validate_checkpoint_ranges(&ranges).is_err());
    }

    #[test]
    fn test_validate_ranges_not_contiguous() {
        let ranges = vec![
            CheckpointRangeType::Regular(CheckpointRange {
                height_start: 0,
                height_end: 999,
                checkpoint_interval: 100,
            }),
            CheckpointRangeType::Regular(CheckpointRange {
                height_start: 1500, // Should be 1000
                height_end: 2499,
                checkpoint_interval: 100,
            }),
        ];
        assert!(validate_checkpoint_ranges(&ranges).is_err());
    }

    #[test]
    fn test_validate_ranges_overlapping() {
        let ranges = vec![
            CheckpointRangeType::Regular(CheckpointRange {
                height_start: 0,
                height_end: 1000,
                checkpoint_interval: 100,
            }),
            CheckpointRangeType::Regular(CheckpointRange {
                height_start: 500, // Overlaps with first range
                height_end: 1500,
                checkpoint_interval: 100,
            }),
        ];
        assert!(validate_checkpoint_ranges(&ranges).is_err());
    }

    #[test]
    fn test_parse_genesis_range() {
        let range = parse_single_range("0", 0).unwrap();
        assert!(matches!(range, CheckpointRangeType::Genesis(0)));
    }

    #[test]
    fn test_parse_genesis_range_non_zero() {
        let range = parse_single_range("1000", 0).unwrap();
        assert!(matches!(range, CheckpointRangeType::Genesis(1000)));
    }

    #[test]
    fn test_genesis_height_start_end() {
        let genesis = CheckpointRangeType::Genesis(500);
        assert_eq!(genesis.height_start(), 500);
        assert_eq!(genesis.height_end(), 500);
    }

    #[test]
    fn test_validate_genesis_only() {
        let ranges = vec![CheckpointRangeType::Genesis(0)];
        assert!(validate_checkpoint_ranges(&ranges).is_ok());
    }

    #[test]
    fn test_validate_genesis_with_regular_range() {
        let ranges = vec![
            CheckpointRangeType::Genesis(0),
            CheckpointRangeType::Regular(CheckpointRange {
                height_start: 1,
                height_end: 1000,
                checkpoint_interval: 100,
            }),
        ];
        assert!(validate_checkpoint_ranges(&ranges).is_ok());
    }

    #[test]
    fn test_validate_genesis_non_zero() {
        let ranges = vec![
            CheckpointRangeType::Genesis(500),
            CheckpointRangeType::Regular(CheckpointRange {
                height_start: 501,
                height_end: 1500,
                checkpoint_interval: 100,
            }),
        ];
        assert!(validate_checkpoint_ranges(&ranges).is_ok());
    }

    #[test]
    fn test_validate_genesis_not_first_fails() {
        let ranges = vec![
            CheckpointRangeType::Regular(CheckpointRange {
                height_start: 0,
                height_end: 999,
                checkpoint_interval: 100,
            }),
            CheckpointRangeType::Genesis(1000),
        ];
        assert!(validate_checkpoint_ranges(&ranges).is_err());
    }

    #[test]
    fn test_validate_genesis_before_regular_range_fails() {
        let ranges = vec![
            CheckpointRangeType::Genesis(1000),
            CheckpointRangeType::Regular(CheckpointRange {
                height_start: 500,
                height_end: 1500,
                checkpoint_interval: 100,
            }),
        ];
        assert!(validate_checkpoint_ranges(&ranges).is_err());
    }

    #[test]
    fn test_parse_checkpoint_ranges_with_genesis() {
        let ranges = parse_checkpoint_ranges("0;1,1000,100").unwrap();
        assert_eq!(ranges.len(), 2);
        assert!(matches!(&ranges[0], CheckpointRangeType::Genesis(0)));
        if let CheckpointRangeType::Regular(range) = &ranges[1] {
            assert_eq!(range.height_start, 1);
            assert_eq!(range.height_end, 1000);
        } else {
            panic!("Expected second range to be regular");
        }
    }

    #[test]
    fn test_checkpoint_range_type_equality() {
        let genesis1 = CheckpointRangeType::Genesis(100);
        let genesis2 = CheckpointRangeType::Genesis(100);
        let genesis3 = CheckpointRangeType::Genesis(200);

        assert_eq!(genesis1, genesis2);
        assert_ne!(genesis1, genesis3);
    }
}
