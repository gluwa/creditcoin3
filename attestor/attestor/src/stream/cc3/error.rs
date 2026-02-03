#[derive(Debug)]
pub enum Error {
    SubxtError(subxt::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::SubxtError(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for Error {}
