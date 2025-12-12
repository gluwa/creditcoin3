#[derive(Debug)]
pub enum Error {
    CC3Error(crate::chain_listener::cc3::Error),
    Pool(super::pool::Error),
    SubxtError(subxt::Error),
    BlsError(bls_signatures::Error),
    InvalidAttestation(InvalidCause),
    InvalidBls(Vec<u8>),
    InvalidAttestationEvent,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CC3Error(err) => write!(f, "{err}"),
            Self::Pool(err) => write!(f, "{err}"),
            Self::SubxtError(err) => write!(f, "{err}"),
            Self::BlsError(err) => write!(f, "{err}"),
            Self::InvalidAttestation(cause) => write!(f, "Invalid attestation: {cause}"),
            Self::InvalidBls(bls) => {
                write!(f, "Invalid BLS signature: {}", alloy::hex::encode(bls))
            }
            Self::InvalidAttestationEvent => write!(f, "Invalid attestation event"),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug)]
pub enum InvalidCause {
    Unsupported(attestor_primitives::ChainKey),
    Duplicate,
    InvalidVrf,
    InvalidBls,
    EmptyContinuityProof,
    EmptyPrevDigest,
    InvalidContinuityHeadDigest {
        actual: attestor_primitives::Digest,
        expected: attestor_primitives::Digest,
    },
    // FIXME: cc_client::H256 and attestor_primitives::Digest use two different versions of the
    // same crate so types are incompatible despite having the same signature :P
    InvalidContinuityTailDigest {
        actual: cc_client::H256,
        expected: cc_client::H256,
    },
    InvalidContinuityProof {
        block: attestor_primitives::block::BlockSerializable,
        expected: cc_client::H256,
    },
}

impl std::fmt::Display for InvalidCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(chain_key) => write!(f, "Usupported source chain: {chain_key}"),
            Self::Duplicate => write!(f, "Attestation already exists in the runtime"),
            Self::InvalidVrf => write!(f, "Invalid attestation VRF"),
            Self::InvalidBls => write!(f, "Invalid attestation BLS"),
            Self::EmptyContinuityProof => write!(f, "Empty attestation continuity proof"),
            Self::EmptyPrevDigest => write!(f, "Empty previous digest"),
            Self::InvalidContinuityHeadDigest { actual, expected } => write!(
                f,
                "Invalid continuity proof head digest, expected {expected}, got {actual}"
            ),
            Self::InvalidContinuityTailDigest { actual, expected } => write!(
                f,
                "Invalid continuity proof tail digest, expected {expected}, got {actual}"
            ),
            Self::InvalidContinuityProof { block, expected } => write!(
                f,
                "Invalid continuity proof at block {block:?}, expected previous digest {expected}"
            ),
        }
    }
}
