use attestor_primitives::{AttestorId, Digest, Height};

#[derive(Debug)]
pub enum Error {
    /// Vote came from an attestor not in the current active set.
    Unauthorized(AttestorId, Height),
    /// Vote height is below the latest finalized height or outside the catch-up window.
    InvalidHeight(AttestorId, Height, Height),
    /// Same attestor submitted two different digests at the same height.
    Equivocation(AttestorId, Height),
    /// Vote was already seen as invalid at this digest.
    KnownInvalid(AttestorId, Height, Digest),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unauthorized(a, h) => write!(f, "unauthorized attestor {a} at height {h}"),
            Self::InvalidHeight(a, h, finalized) => write!(
                f,
                "invalid height {h} from {a} (last finalized {finalized})"
            ),
            Self::Equivocation(a, h) => write!(f, "equivocation by {a} at height {h}"),
            Self::KnownInvalid(a, h, d) => write!(f, "{a} re-sent known invalid vote {d} @ {h}"),
        }
    }
}

impl Error {
    pub fn log_error(&self, digest: Digest) {
        match self {
            Self::Unauthorized(..) => tracing::warn!(?digest, %self, "🚫 unauthorized attestor"),
            Self::InvalidHeight(..) => tracing::debug!(?digest, %self, "📛 vote outside window"),
            Self::Equivocation(..) => tracing::error!(?digest, %self, "👯 equivocation"),
            Self::KnownInvalid(..) => tracing::debug!(?digest, %self, "🚯 known invalid"),
        }
    }
}

impl std::error::Error for Error {}
