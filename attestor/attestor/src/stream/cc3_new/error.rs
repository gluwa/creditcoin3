#[derive(Debug)]
pub enum Error {
    Client(cc_client::Error),
    Subxt(subxt::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Client(err) => write!(f, "{err}"),
            Self::Subxt(err) => write!(f, "{err}"),
        }
    }
}
impl std::error::Error for Error {}
