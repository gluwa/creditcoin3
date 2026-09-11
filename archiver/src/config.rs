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

    /// Timeout in seconds for the stream before treating it as stalled.
    #[arg(long, env = "STREAM_TIMEOUT_SECS", default_value = "120")]
    pub stream_timeout_secs: u64,

    /// Path to the sled database directory for root storage.
    #[arg(long, env = "SLED_DB_PATH", default_value = "./data/roots.sled")]
    pub sled_db_path: PathBuf,

    /// HTTP API bind address.
    #[arg(long, env = "API_BIND", default_value = "0.0.0.0:8080")]
    pub api_bind: SocketAddr,

    /// Batch size while catching up: roots are written (and a durability flush requested)
    /// every N blocks. Only affects backfill throughput; tip-following is governed by
    /// `--tip-window` and never needs `1`.
    #[arg(long, env = "FLUSH_EVERY", default_value = "10000")]
    pub flush_every: NonZeroU64,

    /// Blocks from the target (chain head or `--end-height`) within which every root is
    /// written as soon as it is computed, so the API can serve mature roots immediately.
    /// Must comfortably exceed the finalization lag plus head-poll latency (12 s). Durability
    /// flushes at the tip are throttled to about one per second regardless.
    #[arg(long, env = "TIP_WINDOW", default_value = "256")]
    pub tip_window: NonZeroU64,

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

    /// Seconds between `eth_blockNumber` polls that run alongside the `newHeads` subscription.
    /// The poll is the liveness floor: a subscription that acknowledges but stops delivering
    /// headers cannot stall archiving for longer than this.
    #[arg(long, env = "HEAD_POLL_INTERVAL_SECS", default_value = "12")]
    pub head_poll_interval_secs: NonZeroU64,

    /// Deadline in seconds for each RPC call made while (re)establishing the block stream
    /// (subscribe, initial head read, head polls). alloy transports have no default timeout.
    #[arg(long, env = "RPC_TIMEOUT_SECS", default_value = "30")]
    pub rpc_timeout_secs: NonZeroU64,

    /// `/ready` reports 503 when the archive is more than this many blocks behind the mature
    /// target (`source head - finalization lag`). Size it to the chain's block rate: it is the
    /// catch-up debt you are willing to serve proofs from.
    #[arg(long, env = "READY_LAG_BLOCKS", default_value = "1000")]
    pub ready_lag_blocks: u64,

    /// `/ready` reports 503 when the source head has not been sampled successfully for this
    /// many seconds. Distinguishes "the chain is idle" (fresh sample, no new blocks: ready)
    /// from "we lost sight of the chain" (not ready).
    #[arg(long, env = "STALE_AFTER_SECS", default_value = "60")]
    pub stale_after_secs: NonZeroU64,
}
