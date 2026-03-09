#[derive(Debug)]
pub enum Error {
    SubxtError(subxt::Error),
    Client(cc_client::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SubxtError(err) => write!(f, "{err}"),
            Self::Client(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for Error {}
