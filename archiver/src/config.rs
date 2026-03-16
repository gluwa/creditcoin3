//! Archiver configuration via CLI flags and environment variables.

use std::net::SocketAddr;
use std::num::{NonZeroU64, NonZeroUsize};
use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "archiver",
    about = "Source chain archiver — fetches blocks, computes merkle roots, serves data over HTTP"
)]
pub struct Config {
    /// RPC endpoint (WebSocket) for block fetching and subscriptions.
    #[arg(long, env = "RPC_URL", required = true)]
    pub rpc_url: String,

    /// Block height to start from (ignored if the database already has progress).
    #[arg(long, env = "START_HEIGHT", default_value = "0")]
    pub start_height: u64,

    /// Block height to stop at (inclusive). When set, the archiver will stop
    /// after processing this block and exit. Omit to follow the chain tip.
    #[arg(long, env = "END_HEIGHT")]
    pub end_height: Option<u64>,

    /// Finalization lag — number of blocks behind the tip to consider finalized.
    #[arg(long, env = "FINALIZATION_LAG", default_value = "64")]
    pub finalization_lag: u64,

    /// Maximum concurrent block fetch tasks (IO-bound).
    #[arg(long, env = "MAX_CONCURRENCY", default_value = "8")]
    pub max_concurrency: NonZeroUsize,

    /// Maximum parallel merkle root computations (CPU-bound).
    #[arg(long, env = "MAX_PARALLELISM", default_value = "4")]
    pub max_parallelism: NonZeroUsize,

    /// Path to the sled database directory for root storage.
    #[arg(long, env = "SLED_DB_PATH", default_value = "./data/roots.sled")]
    pub sled_db_path: PathBuf,

    /// HTTP API bind address.
    #[arg(long, env = "API_BIND", default_value = "0.0.0.0:8080")]
    pub api_bind: SocketAddr,

    /// How often to flush the sled database to disk (every N blocks).
    #[arg(long, env = "FLUSH_EVERY", default_value = "10000")]
    pub flush_every: NonZeroU64,
}
