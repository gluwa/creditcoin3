//! Archiver configuration via CLI flags and environment variables.

use std::net::SocketAddr;
use std::num::{NonZeroU64, NonZeroUsize};
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use url::Url;

#[derive(Parser, Debug)]
#[command(
    name = "archiver",
    about = "Source chain archiver — fetches blocks, computes merkle roots, serves data over HTTP",
    // When a subcommand (e.g. `compare`) is given, the streaming-mode required
    // args below (`--rpc-http`, `--rpc-ws`) are not required. With no subcommand
    // the archiver runs in streaming mode exactly as before.
    subcommand_negates_reqs = true
)]
pub struct Config {
    /// Optional subcommand. When omitted, the archiver runs in its normal
    /// streaming mode using the flags below.
    #[command(subcommand)]
    pub command: Option<Command>,

    /// HTTP RPC endpoint(s) for block fetching. Repeat the flag (or pass a
    /// comma-separated list via the `RPC_HTTP` env var) to configure multiple
    /// endpoints; block fetches are then distributed across them round-robin to
    /// spread request load. At least one endpoint is required.
    #[arg(
        long = "rpc-http",
        env = "RPC_HTTP",
        alias = "rpc-url",
        required = true,
        value_delimiter = ','
    )]
    pub rpc_http: Vec<Url>,

    /// WebSocket RPC endpoint for new-head subscriptions.
    /// Required for the root stream to follow the chain tip (but not for
    /// subcommands such as `compare`, which is why this is modelled as optional
    /// and validated at runtime in streaming mode).
    #[arg(long, env = "RPC_WS", required = true)]
    pub rpc_ws: Option<Url>,

    /// Creditcoin3 RPC (WebSocket). `CC3_RPC_URL` or `--cc3-rpc-url` (CLI overrides env; not in YAML).
    #[arg(long, default_value = "ws://localhost:9944", env = "CC3_RPC_URL")]
    pub cc3_rpc_url: String,

    /// The chain key corresponding to the source chain supported by this archiver. Used for fetching
    /// on-chain maturity strategy
    #[arg(long, env = "CHAIN_KEY")]
    pub chain_key: Option<u64>,

    /// Block height to start from (ignored if the database already has progress).
    #[arg(long, env = "START_HEIGHT", default_value = "0")]
    pub start_height: u64,

    /// Block height to stop at (inclusive). When set, the archiver will stop
    /// after processing this block and exit. Omit to follow the chain tip.
    #[arg(long, env = "END_HEIGHT")]
    pub end_height: Option<u64>,

    /// Maximum concurrent block fetch tasks (IO-bound). Throughput scales
    /// roughly linearly with this until the RPC providers rate-limit, so raise
    /// it when fetching across several distinct providers. This is async IO
    /// concurrency and is independent of `--max-parallelism` (CPU-bound merkle).
    #[arg(long, env = "MAX_FETCH_TASKS", default_value = "32")]
    pub max_fetch_tasks: NonZeroUsize,

    /// Maximum number of block merkle-root computations to run in parallel
    /// (CPU-bound, runs on the blocking thread pool). Defaults to the number of
    /// available CPU cores. This is independent of `--max-fetch-tasks`: fetch
    /// tasks are async IO and do not consume CPU cores, so raising fetch
    /// concurrency must not shrink merkle parallelism.
    #[arg(long, env = "MAX_PARALLELISM")]
    pub max_parallelism: Option<NonZeroUsize>,

    /// Maximum block range that can be queried via the /roots API endpoint.
    /// Default is slightly above one checkpoint interval (attestation_interval × checkpoint_interval)
    /// to allow a full checkpoint span plus headroom.
    #[arg(long, env = "MAX_API_RANGE", default_value = "1000")]
    pub max_api_range: u64,

    /// Timeout in seconds for the stream before treating it as stalled.
    #[arg(long, env = "STREAM_TIMEOUT_SECS", default_value = "120")]
    pub stream_timeout_secs: u64,

    /// Path to the sled database directory for root storage.
    #[arg(long, env = "SLED_DB_PATH", default_value = "./data/roots.sled")]
    pub sled_db_path: PathBuf,

    /// HTTP API bind address.
    #[arg(long, env = "API_BIND", default_value = "0.0.0.0:8080")]
    pub api_bind: SocketAddr,

    /// How often to flush the sled database to disk (every N blocks).
    #[arg(long, env = "FLUSH_EVERY", default_value = "10000")]
    pub flush_every: NonZeroU64,

    /// Finalization lag: number of blocks behind the chain tip to consider finalized.
    /// By default the archiver will use the on-chain finalization lag for this source
    /// chain as registered on Creditcoin. The default will be correct in most cases.
    ///
    /// Set to 0 for chains with instant finality. For chains with probabilistic
    /// finality, set this to the expected number of confirmation blocks.
    #[arg(long, env = "FINALIZATION_LAG")]
    pub finalization_lag_override: Option<u64>,

    /// Scan the database for gaps and fill them before resuming normal operation.
    #[arg(long, default_value_t = false)]
    pub backfill: bool,

    /// Before following the chain tip, run a fast out-of-order catch-up over the
    /// historical range (resume height → finalized tip). Block roots are fetched
    /// with full fetch concurrency and written to the database as they complete,
    /// rather than strictly in height order, so throughput is bounded by the
    /// average RPC latency instead of the slowest in-flight block. It is
    /// idempotent and restart-safe: only missing heights are fetched, so an
    /// interrupted run simply resumes. Best for large historical backfills.
    #[arg(long, default_value_t = false)]
    pub fast_catchup: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Compare merkle roots between two sled databases over a height range.
    ///
    /// Walks both databases in lockstep and reports every height where the
    /// stored roots differ or where a height is present in only one database.
    /// Exits non-zero if any mismatch is found.
    Compare(CompareArgs),
}

#[derive(Args, Debug)]
pub struct CompareArgs {
    /// Path to the first sled database (reported as "db-a").
    #[arg(long)]
    pub db_a: PathBuf,

    /// Path to the second sled database (reported as "db-b").
    #[arg(long)]
    pub db_b: PathBuf,

    /// Start height (inclusive).
    #[arg(long, default_value = "0")]
    pub from: u64,

    /// End height (inclusive). Defaults to the highest height present in both
    /// databases.
    #[arg(long)]
    pub to: Option<u64>,
}
