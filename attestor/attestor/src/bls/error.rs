#[derive(Debug)]
pub enum Error {
    Client(cc_client::Error),
    InvalidBls(cc_client::AccountId32),
    Unregistered(cc_client::AccountId32),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Client(err) => write!(f, "{err}"),
            Self::InvalidBls(attestor_id) => {
                write!(f, "Attestor {attestor_id} has invalid BLS pubkey")
            }
            Self::Unregistered(attestor_id) => {
                write!(f, "Attestor {attestor_id} is not registered")
            }
        }
    }
}

impl std::error::Error for Error {}
