# Checkpoint Builder

A tool for generating block Checkpoints from block root data. It reads block roots from either a local Sled database or an archiver HTTP API, processes blocks in intervals, and outputs checkpoint data (block number and digest) to a CSV file.

## Overview

The Checkpoint Builder performs the following:

1. Reads block root data from a configured source (Sled database or archiver HTTP API)
2. Reads blocks starting from a configured block height
3. Computes checkpoint digests at a configured checkpoint size
4. Batches checkpoints and writes them to a timestamped CSV file
5. Supports graceful shutdown on CTRL-C with automatic file flushing

### Sources

**Sled** — reads from a local Sled database where:
- **Key**: block height as big-endian u64 bytes (8 bytes)
- **Value**: block root digest (32 bytes)

**Archiver** — reads from an archiver HTTP API that exposes block root data via REST endpoints (`/roots`, `/roots/latest`, `/status`).

The output CSV format is:
```
0,0x1234...
100,0x5678...
200,0xabcd...
```

## Prerequisites

- Rust 1.70+ (uses workspace configuration)
- A Sled database containing block root data **or** access to an archiver HTTP API

## Configuration

The tool uses **subcommands** to select the block root source (`sled` or `archiver`). Each subcommand has source-specific options plus shared common options. Configuration can be provided via **command-line arguments** or **environment variables** via .env files. CLI arguments take precedence over environment variables.

### Source-Specific Options

**`sled` subcommand:**

| Option | CLI Argument | Environment Variable | Default | Description |
|--------|--------------|----------------------|---------|-------------|
| Sled DB Path | `--sled-db-path` | `SLED_DB_PATH` | - | **Required**. Path to the Sled database directory |

**`archiver` subcommand:**

| Option | CLI Argument | Environment Variable | Default | Description |
|--------|--------------|----------------------|---------|-------------|
| Archiver URL | `--archiver-url` | `ARCHIVER_URL` | - | **Required**. Base URL of the archiver HTTP API (e.g. `http://localhost:8080`) |

### Common Options (both subcommands)

| Option | CLI Argument | Environment Variable | Default | Description |
|--------|--------------|----------------------|---------|-------------|
| Checkpoint Ranges | `--checkpoint-ranges` | `CHECKPOINT_RANGES` | - | **Required** unless `--dry-run`. Ranges in format `genesis_height` or `start,end,interval;...` |
| Starting Digest | `--starting-digest` | `STARTING_DIGEST` | - | 32-byte hex genesis digest. Required when first range is not a genesis checkpoint |
| Commit Interval | `--checkpoint-flush-interval` | `CHECKPOINT_FLUSH_INTERVAL` | `20` | Checkpoints to batch before writing to CSV |
| Validate Database | `--validate-database` | `VALIDATE_DATABASE` | `false` | Check that all blocks in the specified ranges exist in the source before processing |
| Output File | `--output-file` | `OUTPUT_FILE` | `checkpoints.csv` | Output CSV path (timestamp will be appended) |
| Dry Run | `--dry-run` | `DRY_RUN` | `false` | Print the available block range in the source and exit without generating checkpoints |

### Checkpoint Ranges Format

The `CHECKPOINT_RANGES` parameter defines checkpoint ranges using a semicolon-separated list. Each range can be either:

**Genesis Checkpoint:** A single checkpoint at the starting block height
```
CHECKPOINT_RANGES="100"
```

**Regular Range:** Multiple checkpoints with a specified interval
```
CHECKPOINT_RANGES="start,end,interval"
```

**Multiple Ranges:** Genesis checkpoint followed by regular ranges
```
CHECKPOINT_RANGES="100;101,1000,100;1001,5000,500"
```

**Examples:**

*Starting from block 1 with regular ranges:*
```
CHECKPOINT_RANGES="1,1000,100;1001,5000,500;5001,10000,1000"
```
Creates:
- Checkpoints every 100 blocks for blocks 1-1000 (10 checkpoints)
- Checkpoints every 500 blocks for blocks 1001-5000 (8 checkpoints)
- Checkpoints every 1000 blocks for blocks 5001-10000 (5 checkpoints)

*Starting from block 100 with genesis + regular ranges:*
```
CHECKPOINT_RANGES="100;101,999,100;1000,4999,500"
```
Creates:
- Genesis checkpoint at block 100
- Checkpoints every 100 blocks for blocks 101-999
- Checkpoints every 500 blocks for blocks 1000-4999

**Validation Rules:**
1. At least one range is required
2. Only the **first** range can be a genesis checkpoint (single height value)
3. Regular ranges must have `height_start <= height_end`
4. Each `checkpoint_interval` must be > 0
5. Block count in each regular range must be divisible by its interval
6. Regular ranges must be contiguous: `range[i].height_end + 1 == range[i+1].height_start`
7. Ranges must be ordered by height (ascending)
8. Block `0` can only be provided in a genesis range

### Genesis Block and Starting Digest

The tool supports two approaches to initialize checkpoint generation:

**Approach 1: Genesis Checkpoint (start from known genesis block)**

Use the first range as a genesis checkpoint (e.g., `"100"`). This creates a single checkpoint at that block height, derived from the block at that height in the database. This is useful when restarting from a known good state.

Example:
```
CHECKPOINT_RANGES="100;101,1000,100"
```

**Approach 2: Starting Digest (resume from a checkpoint)**

Provide the `--starting-digest` parameter when the first range is **not** a genesis range. This digest represents the checkpoint at block `(range_start - 1)`, allowing you to resume checkpoint generation from a previous state.

Example:
```
CHECKPOINT_RANGES="101,1000,100"
STARTING_DIGEST="0x1234abcd..."
```

The tool will use the provided digest as the genesis checkpoint at block 100, then start generating new checkpoints from block 101 onwards.

**Rules:**
- If the first range is a genesis checkpoint (single height), **do NOT provide** `--starting-digest`
- If the first range is a regular range (start,end,interval), **you MUST provide** `--starting-digest`
- If starting from block 0, **you MUST provide an initial genesis range** with regular ranges after that like so: `0;1,1000,100;1001,2000,50`

### Environment Variables Setup

Create a `.env` file in the checkpoint-builder directory:

```bash
cp .env.example .env
# Edit .env with your configuration
```

Example `.env` (sled source, genesis approach):
```dotenv
SLED_DB_PATH="./block_roots_db"
CHECKPOINT_RANGES="0;1,10000,100"
CHECKPOINT_FLUSH_INTERVAL=20
OUTPUT_FILE="checkpoints.csv"
```

Example `.env` (archiver source):
```dotenv
ARCHIVER_URL="http://localhost:8080"
CHECKPOINT_RANGES="0;1,10000,100"
CHECKPOINT_FLUSH_INTERVAL=20
OUTPUT_FILE="checkpoints.csv"
```

## Usage

### Basic Usage (using .env file)

```bash
# From the checkpoint-builder directory, using sled source
cargo run -p checkpoint-builder --release -- sled

# Or using archiver source
cargo run -p checkpoint-builder --release -- archiver
```

### Sled Source — Genesis Checkpoint Approach

```bash
# Generate checkpoints starting from genesis block at height 0
cargo run -p checkpoint-builder --release -- sled \
  --sled-db-path ./my_block_roots_db \
  --checkpoint-ranges "0;1,999,100;1000,4999,500" \
  --output-file my_checkpoints.csv
```

### Sled Source — Starting Digest Approach (Resume from Checkpoint)

```bash
# Resume from block 50000 with a known checkpoint digest
cargo run -p checkpoint-builder --release -- sled \
  --sled-db-path ./block_roots_db \
  --starting-digest 0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef \
  --checkpoint-ranges "50000,500000,1000"
```

The `--starting-digest` represents the checkpoint at block 49999, allowing the tool to continue the checkpoint chain from block 50000 onwards.

### Archiver Source

```bash
# Generate checkpoints from an archiver HTTP API
cargo run -p checkpoint-builder --release -- archiver \
  --archiver-url http://localhost:8080 \
  --checkpoint-ranges "0;1,999,100;1000,4999,500" \
  --output-file my_checkpoints.csv
```

### Dry Run (inspect available block range)

Use `--dry-run` to query the source's first and last available block without generating any checkpoints. `--checkpoint-ranges` is not required in this mode.

```bash
# Sled source
cargo run -p checkpoint-builder --release -- sled \
  --sled-db-path ./block_roots_db \
  --dry-run

# Archiver source
cargo run -p checkpoint-builder --release -- archiver \
  --archiver-url http://localhost:8080 \
  --dry-run
```

Output:
```
Available range: 0 to 99999
```

### Enabling Source Validation

By default the tool doesn't check that all blocks in the specified ranges exist in the source before processing. To enable the checks use the `--validate-dabase` flags:

```bash
cargo run -p checkpoint-builder --release -- archiver \
  --archiver-url http://localhost:8080 \
  --validate-database \
  --checkpoint-ranges "0;1,999,100;1000,4999,500"
```

Keep in mind that depending on the size of the intervals to validate the operation could take a long time!

### Using Built Binary

```bash
# Build once
cargo build -p checkpoint-builder --release

# Run with sled source
./target/release/checkpoint-builder sled \
  --sled-db-path ./block_roots_db \
  --checkpoint-ranges "0;1,100000,100"

# Run with archiver source
./target/release/checkpoint-builder archiver \
  --archiver-url http://localhost:8080 \
  --checkpoint-ranges "0;1,100000,100"

# Run with starting digest (works with either source)
./target/release/checkpoint-builder sled \
  --sled-db-path ./block_roots_db \
  --starting-digest 0x... \
  --checkpoint-ranges "1001,100000,100"
```

## Output

### CSV File Format

The tool writes checkpoints to a CSV file with a timestamp suffix:
- **Naming**: `{output_file}_{timestamp}.csv`
- **Example**: `checkpoints_20260225T154718.csv`
- **Timestamp Format**: `YYYYMMDDTHHMMSS` (ISO 8601 compact)

### Sample Output

```csv
100,0x5768ab48e1ce21a00b7cdf3a7c3b6a1234567890abcdef1234567890abcdef76f3
200,0xdb34cd5678901234567890abcdef12345678901234567890abcdef12345e4f3
300,0x86c1ef90abcdef1234567890abcdef1234567890abcdef1234567890abcd5d6f
```

### Log Output

The tool logs progress using `tracing` (default level: `INFO`):

```
2026-02-25T15:47:18.203821Z  INFO Starting checkpoint generator with config: {...}
2026-02-25T15:47:18.204531Z  INFO Source first entry at block 0 with digest 0x1234...
2026-02-25T15:47:18.204714Z  INFO Spawning CSV sink task with output file: checkpoints_20260225T154718.csv and commit interval: 20
2026-02-25T15:47:18.205000Z  INFO Processing range 1/2: blocks 0 to 999 with interval 100
2026-02-25T15:47:19.114647Z  INFO Sending checkpoint for block 100 with digest 0x1234... to output file
2026-02-25T15:47:19.118189Z  INFO Successfully wrote 20 checkpoints to CSV file: checkpoints_20260225T154718.csv
```

## Graceful Shutdown

The tool handles **CTRL-C** (SIGINT) gracefully:

1. Press `CTRL-C` to initiate shutdown
2. The main block processing task exits immediately
3. Any buffered checkpoints in memory are flushed to the CSV file
4. The CSV file is closed cleanly
5. The process exits

This ensures **no data loss** even if you interrupt the tool during execution.

```bash
$ cargo run -p checkpoint-builder --release -- sled --sled-db-path ./block_roots_db --checkpoint-ranges "0;1,100000,100"
# ... processing ...
^C
2026-02-25T15:50:00Z  WARN Received CTRL-C, initiating graceful shutdown...
2026-02-25T15:50:00Z  INFO Waiting for remaining checkpoints to be written to output file...
2026-02-25T15:50:00Z  INFO Checkpoint generation completed
```

## Logging Control

Control log verbosity with the `RUST_LOG` environment variable:

```bash
# Only INFO and above
RUST_LOG=info cargo run -p checkpoint-builder --release -- sled

# Debug level
RUST_LOG=debug cargo run -p checkpoint-builder --release -- archiver

# Silence most logs, only errors
RUST_LOG=error cargo run -p checkpoint-builder --release -- sled

# Per-module control
RUST_LOG=checkpoint_builder=debug,stream_eth=info cargo run -p checkpoint-builder --release -- sled
```

## Architecture

### Component Overview

- **Main**: Orchestrates block processing, source selection, and graceful shutdown
- **Config**: Parses CLI subcommands, arguments, and environment variables
- **Source/Sled**: Reads block root data from a local Sled database
- **Source/Archiver**: Reads block root data from an archiver HTTP API (with automatic batching for large ranges)
- **Sink/CSV**: Manages buffered writing to CSV with persistent file handle
- **Digest Computation**: Incremental Merkle root hashing using keccak256

### Key Design Features

- **Pluggable Sources**: Supports both local Sled databases and remote archiver HTTP APIs via the `RootSource` trait
- **Automatic Batching**: The archiver source automatically splits large range requests into batches of up to 99,999 blocks
- **Source Validation**: Optionally validates that all required blocks exist before processing begins
- **Persistent File Handle**: The CSV writer is opened once and reused across all writes (vs. reopening per batch)
- **Batching**: Checkpoints are batched in memory before writing to reduce I/O
- **Async/Tokio**: Uses async/await for concurrent CSV writing
- **Graceful Shutdown**: CTRL-C triggers channel closure, allowing sink to flush remaining data
- **Timestamp Naming**: Output files automatically include creation timestamp for easy timestamped backups

## Troubleshooting

### "Sled database path does not exist"
The specified Sled database path doesn't exist. Check that:
- The path is correct
- The database directory was created by the block root ingestion process

### "Failed to open sled database"
Verify that:
- The path points to a valid Sled database directory
- You have read permissions on the directory
- The database is not corrupted

### Archiver source connection errors
If using the `archiver` subcommand and seeing HTTP errors:
- Verify the `--archiver-url` is correct and the archiver service is running
- Check network connectivity to the archiver
- The archiver API has a 60-second request timeout per batch

### "Database is empty or no blocks at starting height"
The source doesn't contain blocks at the specified starting height. Possible causes:
- The database/archiver hasn't been populated yet
- The starting block is beyond the available data
- The database was created with different block ranges

### CSV file is empty or missing
- Check that the correct output path is writable
- Look for the timestamped filename (not the original `OUTPUT_FILE` name)
- Press CTRL-C to flush any pending writes
