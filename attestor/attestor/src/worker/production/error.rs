#[derive(Debug)]
pub enum Error {
    Attestation(super::stream::attestation::Error),
    CC3(super::stream::cc3::Error),
    Interrupt,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Attestation(err) => write!(f, "{err}"),
            Error::CC3(err) => write!(f, "{err}"),
            _ => todo!(),
        }
    }
}

impl std::error::Error for Error {}
