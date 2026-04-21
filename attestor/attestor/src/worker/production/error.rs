#[derive(Debug)]
pub enum Error {
    CC3(stream::cc3::Error),
    Bls(crate::bls::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CC3(err) => write!(f, "{err}"),
            Self::Bls(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for Error {}
