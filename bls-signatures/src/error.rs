use thiserror_no_std::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Size mismatch")]
    SizeMismatch,
    #[error("Io error")]
    Io,
    #[error("Group decode error")]
    GroupDecode,
    #[error("Curve decode error")]
    CurveDecode,
    #[error("Prime field decode error")]
    FieldDecode,
    #[error("Invalid Private Key")]
    InvalidPrivateKey,
    #[error("Zero sized input")]
    ZeroSizedInput,
}

impl From<acid_io::Error> for Error {
    fn from(_: acid_io::Error) -> Self {
        Error::Io
    }
}
