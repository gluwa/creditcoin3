#[derive(Debug)]
pub enum Error {
    CC3(crate::stream_legacy::cc3::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CC3(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for Error {}
