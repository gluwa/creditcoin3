//! Archiver configuration via CLI flags and environment variables.

use std::net::SocketAddr;
use std::num::{NonZeroU64, NonZeroUsize};
use std::path::PathBuf;

use clap::Parser;
use url::Url;

#[derive(Parser, Debug)]
#[command(
    name = "archiver",
    about = "Source chain archiver — fetches blocks, computes merkle roots, serves data over HTTP"
)]
pub struct Config {
    /// HTTP RPC endpoint for block fetching.
    #[arg(long, env = "RPC_HTTP", alias = "rpc-url", required = true)]
    pub rpc_http: Url,

    /// WebSocket RPC endpoint for new-head subscriptions.
    /// Required for the root stream to follow the chain tip.
    #[arg(long, env = "RPC_WS", required = true)]
    pub rpc_ws: Url,

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

    /// Maximum concurrent block fetch tasks (IO-bound).
    #[arg(long, env = "MAX_FETCH_TASKS", default_value = "8")]
    pub max_fetch_tasks: NonZeroUsize,

    /// Maximum block range that can be queried via the /roots API endpoint.
    /// Default is slightly above one checkpoint interval (attestation_interval × checkpoint_interval)
    /// to allow a full checkpoint span plus headroom.
    #[arg(long, env = "MAX_API_RANGE", default_value = "1000")]
    pub max_api_range: u64,

    /// Maximum number of in-flight `/roots` API requests served concurrently.
    /// Requests beyond this limit are rejected immediately with HTTP 429 rather
    /// than queued, protecting the archiver from range-scan fan-out overload.
    #[arg(long, env = "MAX_API_CONCURRENCY", default_value = "16")]
    pub max_api_concurrency: NonZeroUsize,

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
}
