#[derive(Debug)]
pub enum Error {
    Client(cc_client::Error),
    Subxt(subxt::Error),
    EndOfStream,
    BlockHash(attestor_primitives::Height),
    /// No finalized block arrived within the progress deadline while seeding.
    NoProgress(std::time::Duration),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Client(err) => write!(f, "{err}"),
            Self::Subxt(err) => write!(f, "{err}"),
            Self::EndOfStream => write!(f, "Unexpected end of stream"),
            Self::BlockHash(n) => write!(f, "Failed to retrieve hash for block {n}"),
            Self::NoProgress(after) => {
                write!(f, "No finalized block received within {after:?}")
            }
        }
    }
}
impl std::error::Error for Error {}
