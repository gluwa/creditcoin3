//! Single error type for the v2 attestor. No `Interrupt<E>` cancellation channel — cancellation
//! flows through a `tokio_util::sync::CancellationToken`. Tasks return ordinary `Result<(), Error>`.

#[derive(Debug)]
pub enum Error {
    /// Initialization-time failure: misconfiguration, chain setup, etc. Aborts startup.
    Init(anyhow::Error),

    /// CC3 RPC client error.
    Rpc(cc_client::Error),

    /// CC3 event stream error.
    Cc3Stream(stream::cc3::Error),

    /// Subxt error.
    Subxt(subxt::Error),

    /// libp2p transport / dial / gossipsub error.
    P2p(anyhow::Error),

    /// BLS verification / aggregation error.
    Bls(bls_signatures::Error),

    /// IO error (used by the api task for binding the metrics listener).
    Io(std::io::Error),

    /// A spawned task panicked or otherwise exited badly.
    TaskJoin(tokio::task::JoinError),

    /// Runtime metadata problem detected by the updater task: undecodable bundled metadata, the
    /// Attestation pallet missing, or its hash drifting from the compiled baseline (binary needs a
    /// rebuild). Returned as an error so the supervisor drains + exits nonzero and k8s reschedules.
    RuntimeMetadata(String),

    /// A long-running task returned `Ok` while the shutdown token was still live. Every task is
    /// meant to loop until `token.cancelled()`; an early clean exit leaves a half-dead pod that
    /// still serves `/metrics` (the k8s-zombie class PR #1034 fixed for v1, reachable here via
    /// `Ok` rather than `Err`). Treated as failure so the supervisor cancels and the pod restarts.
    TaskExitedEarly(&'static str),

    /// Runtime told us a chain key isn't supported.
    ChainKeyNotSupported(attestor_primitives::ChainKey),

    /// `chain_id` from runtime and Eth RPC disagree.
    ChainIdMismatch {
        runtime: attestor_primitives::ChainId,
        rpc: attestor_primitives::ChainId,
    },

    /// Maturity strategy parse / lookup.
    InvalidMaturityStrategy(
        attestor_primitives::ChainKey,
        supported_chains_primitives::Error,
    ),
    NoMaturityDelayForStrategy(supported_chains_primitives::MaturityStrategy),

    /// Attestation interval / sample size missing.
    MissingAttestationInterval(attestor_primitives::ChainKey),
    MissingTargetSampleSize(attestor_primitives::ChainKey),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Init(e) => write!(f, "init: {e}"),
            Self::Rpc(e) => write!(f, "rpc: {e}"),
            Self::Cc3Stream(e) => write!(f, "cc3 stream: {e}"),
            Self::Subxt(e) => write!(f, "subxt: {e}"),
            Self::P2p(e) => write!(f, "p2p: {e}"),
            Self::Bls(e) => write!(f, "bls: {e}"),
            Self::Io(e) => write!(f, "io: {e}"),
            Self::TaskJoin(e) => write!(f, "task join: {e}"),
            Self::RuntimeMetadata(msg) => write!(f, "runtime metadata: {msg}"),
            Self::TaskExitedEarly(name) => {
                write!(f, "task {name} exited before shutdown was requested")
            }
            Self::ChainKeyNotSupported(k) => write!(f, "chain key {k} not supported"),
            Self::ChainIdMismatch { runtime, rpc } => {
                write!(f, "chain_id mismatch: runtime={runtime}, rpc={rpc}")
            }
            Self::InvalidMaturityStrategy(k, e) => {
                write!(f, "invalid maturity strategy for {k}: {e:?}")
            }
            Self::NoMaturityDelayForStrategy(s) => {
                write!(f, "strategy {s:?} has no maturity delay")
            }
            Self::MissingAttestationInterval(k) => {
                write!(f, "missing attestation interval for chain {k}")
            }
            Self::MissingTargetSampleSize(k) => {
                write!(f, "missing target sample size for chain {k}")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<cc_client::Error> for Error {
    fn from(e: cc_client::Error) -> Self {
        Self::Rpc(e)
    }
}
impl From<stream::cc3::Error> for Error {
    fn from(e: stream::cc3::Error) -> Self {
        Self::Cc3Stream(e)
    }
}
impl From<subxt::Error> for Error {
    fn from(e: subxt::Error) -> Self {
        Self::Subxt(e)
    }
}
impl From<bls_signatures::Error> for Error {
    fn from(e: bls_signatures::Error) -> Self {
        Self::Bls(e)
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<tokio::task::JoinError> for Error {
    fn from(e: tokio::task::JoinError) -> Self {
        Self::TaskJoin(e)
    }
}
