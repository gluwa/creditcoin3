#[derive(Debug)]
pub enum Error {
    Join(tokio::task::JoinError),
    Worker(Box<dyn std::error::Error + Sync + Send>),
    Bls(crate::bls::Error),
    Init(anyhow::Error),
    CC3(cc_client::Error),

    MissingAttestationInterval(attestor_primitives::ChainKey),
    MissingCheckpointInterval(attestor_primitives::ChainKey),
    MissingTargetSampleSize(attestor_primitives::ChainKey),

    ChainKeyNotSupported(attestor_primitives::ChainKey),
    ChainIdMisMatch {
        runtime: attestor_primitives::ChainId,
        rpc: attestor_primitives::ChainId,
    },

    InvalidMaturityStrategy(
        attestor_primitives::ChainKey,
        supported_chains_primitives::Error,
    ),
    NoMaturityDelayForStrategy(supported_chains_primitives::MaturityStrategy),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Join(err) => write!(f, "{err}"),
            Error::Worker(err) => write!(f, "{err}"),
            Error::Bls(err) => write!(f, "{err}"),
            Error::Init(err) => write!(f, "Failed to intialize: {err}"),
            Error::CC3(err) => write!(f, "Error polling CC3 stream: {err}"),
            Error::MissingAttestationInterval(chain_key) => write!(
                f,
                "Failed to retrieve attestation interval for chain {chain_key}"
            ),
            Error::MissingCheckpointInterval(chain_key) => write!(
                f,
                "Failed to retrieve checkpoint interval for chain {chain_key}"
            ),
            Error::MissingTargetSampleSize(chain_key) => write!(
                f,
                "Failed to retrieve target sample size for chain {chain_key}"
            ),
            Error::ChainKeyNotSupported(chain_key) => write!(
                f,
                "Chain key not found in supported chains: {chain_key}"
            ),
            Error::ChainIdMisMatch { runtime, rpc } => write!(
                f,
                "Runtime and RPC chain ids do not match: expected {runtime}, got {rpc}"
            ),
            Error::InvalidMaturityStrategy(chain_key, e) => write!(
                f,
                "Initial maturity strategy for chain is invalid ChainKey: {chain_key}, MaturityStrategy: {e:?}"
            ),
            Error::NoMaturityDelayForStrategy(strategy) => write!(
                f,
                "The maturity strategy provided does not have an associated maturity delay. Our EVM implementation requires strategies to have delays. Strategy: {strategy:?}"
            )
        }
    }
}

impl std::error::Error for Error {}
