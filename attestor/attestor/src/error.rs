#[derive(Debug)]
pub enum Error {
    JoinError(tokio::task::JoinError),
    WorkerError(Box<dyn std::error::Error + Sync + Send>),
    Init(anyhow::Error),
    CC3Error(cc_client::Error),
    InitError(Box<dyn std::error::Error + Sync + Send>),
    MissingAttestationInterval(attestor_primitives::ChainKey),
    MissingCheckpointInterval(attestor_primitives::ChainKey),
    MissingTargetSampleSize(attestor_primitives::ChainKey),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::JoinError(err) => write!(f, "{err}"),
            Error::WorkerError(err) => write!(f, "{err}"),
            Error::Init(err) => write!(f, "Failed to intialize: {err}"),
            Error::CC3Error(err) => write!(f, "Error starting CC3 source chain listener: {err}"),
            Error::InitError(err) => write!(f, "Failed to initialize attestor: {err}"),
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
        }
    }
}

impl std::error::Error for Error {}
