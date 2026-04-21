#[derive(Debug)]
pub enum Error {
    InvalidAttestation(InvalidCause),
    PublishError(
        attestor_primitives::Height,
        attestor_primitives::Digest,
        libp2p::gossipsub::PublishError,
    ),
    Transport(libp2p::TransportError<std::io::Error>),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidAttestation(cause) => write!(f, "Invalid attestation: {cause}"),
            Self::PublishError(height, digest, err) => write!(
                f,
                "Failed to publish local attestation \
                and height {height} \
                with digest {digest}: \
                {err}"
            ),
            Self::Transport(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug)]
pub enum InvalidCause {
    InvalidBls(attestor_primitives::Digest),
    Unregistered(attestor_primitives::AttestorId),
    Unsupported(attestor_primitives::ChainKey),
}

impl std::fmt::Display for InvalidCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBls(digest) => {
                write!(f, "Invalid BLS signature for attestation {digest:?}")
            }
            Self::Unregistered(attestor_id) => {
                write!(f, "Attestor {attestor_id} is not registered on-chain")
            }
            Self::Unsupported(chain_key) => write!(f, "Unsupported chain key {chain_key}"),
        }
    }
}
