#[derive(Debug)]
pub enum Error {
    EthError(crate::chain_listener::eth::Error),
    CC3Error(crate::chain_listener::cc3::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::EthError(err) => write!(f, "{err}"),
            Error::CC3Error(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for Error {}
