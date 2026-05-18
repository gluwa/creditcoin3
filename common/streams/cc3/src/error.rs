#[derive(Debug)]
pub enum Error {
    Client(cc_client::Error),
    EndOfStream,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Client(err) => write!(f, "{err}"),
            Self::EndOfStream => write!(f, "Unexpected end of stream"),
        }
    }
}
impl std::error::Error for Error {}
