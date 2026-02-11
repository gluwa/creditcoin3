#[derive(Debug)]
pub enum Error {
    Attestation(crate::stream::attestation::Error),
    CC3(crate::stream::cc3::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Attestation(err) => write!(f, "{err}"),
            Self::CC3(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for Error {}
