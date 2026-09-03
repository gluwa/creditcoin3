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

    /// USC write-ability (cross-chain message attestation) failure: outbox resolution, EVM RPC,
    /// signing-key derivation, or message-vote gossip setup.
    WriteAbility(anyhow::Error),

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

    /// Attestation interval / sample size / max-catchup fetch failed at startup.
    MissingAttestationInterval(attestor_primitives::ChainKey),
    MissingTargetSampleSize(attestor_primitives::ChainKey),
    MissingMaxCatchup(attestor_primitives::ChainKey),

    /// Ctrl+C / SIGTERM arrived while we were still in the synchronous startup phase (waiting on
    /// RPC endpoints or election). Not a failure — `run` maps it to a clean exit.
    ShutdownDuringStartup,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The `anyhow`-wrapping variants honour the alternate flag: `{:#}` on an `anyhow::Error`
        // prints its whole `Context` chain, plain `{}` prints only the outermost layer. Without
        // forwarding the flag, a caller asking for the chain (`tracing::error!(err = format!("{e:#}"))`)
        // silently got the one-line form, because a `write!(f, "{e}")` here formats the inner error
        // with its own default flags regardless of ours (bugbot). Non-anyhow variants have no chain,
        // so they are unaffected.
        macro_rules! chained {
            ($f:expr, $prefix:literal, $e:expr) => {
                if $f.alternate() {
                    write!($f, concat!($prefix, ": {:#}"), $e)
                } else {
                    write!($f, concat!($prefix, ": {}"), $e)
                }
            };
        }
        match self {
            Self::Init(e) => chained!(f, "init", e),
            Self::Rpc(e) => write!(f, "rpc: {e}"),
            Self::Cc3Stream(e) => write!(f, "cc3 stream: {e}"),
            Self::Subxt(e) => write!(f, "subxt: {e}"),
            Self::P2p(e) => chained!(f, "p2p", e),
            Self::WriteAbility(e) => chained!(f, "write-ability", e),
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
            Self::MissingMaxCatchup(k) => {
                write!(f, "failed to fetch max catchup for chain {k}")
            }
            Self::ShutdownDuringStartup => write!(f, "shutdown requested during startup"),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `{:#}` on an `anyhow`-wrapping variant must reach the *cause*, not just the outermost
    /// context. The supervisor logs task failures that way so an operator (and the CI error gate)
    /// can tell a deliberate outage recovery from a real defect; a `Display` impl that formatted the
    /// inner error with plain `{}` made that request silently a no-op.
    #[test]
    fn alternate_display_prints_the_anyhow_chain() {
        let inner = anyhow::anyhow!("eth_getLogs from 100 to 200 failed")
            .context("outbox poll stalled (no progress) 10 times in a row")
            .context("outbox listener died");
        let err = Error::WriteAbility(inner);

        let alternate = format!("{err:#}");
        assert!(alternate.starts_with("write-ability: outbox listener died"));
        assert!(
            alternate.contains("outbox poll stalled (no progress)"),
            "the cause the CI gate keys on must survive: {alternate}"
        );
        assert!(
            alternate.contains("eth_getLogs"),
            "root cause missing: {alternate}"
        );

        // Plain Display stays one-line, so callers that want the short form are unaffected.
        let plain = format!("{err}");
        assert_eq!(plain, "write-ability: outbox listener died");
    }
}
