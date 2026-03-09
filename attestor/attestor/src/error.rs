#[derive(Debug)]
pub enum Error {
    JoinError(tokio::task::JoinError),
    WorkerError(Box<dyn std::error::Error + Sync + Send>),
    InitError(anyhow::Error),
    RpcError(cc_client::Error),
    CC3Error(crate::stream::cc3::Error),
    AttestationError(crate::stream::attestation::Error),
    MissingAttestationInterval(attestor_primitives::ChainKey),
    MissingCheckpointInterval(attestor_primitives::ChainKey),
    MissingTargetSampleSize(attestor_primitives::ChainKey),
    ChainKeyNotSupported(attestor_primitives::ChainKey),
    InvalidMaturityStrategy(
        attestor_primitives::ChainKey,
        supported_chains_primitives::Error,
    ),
    NoMaturityDelayForStrategy(supported_chains_primitives::MaturityStrategy),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::JoinError(err) => write!(f, "{err}"),
            Error::WorkerError(err) => write!(f, "{err}"),
            Error::InitError(err) => write!(f, "Failed to intialize: {err}"),
            Error::CC3Error(err) => write!(f, "Error polling CC3 stream: {err}"),
            Error::AttestationError(err) => write!(f, "Error polling attestation stream: {err}"),
            Error::RpcError(err) => write!(f, "Error calling CC3 client: {err}"),
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
