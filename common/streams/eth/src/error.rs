#[derive(Debug)]
pub enum Error {
    Client(eth::Error),
    Task(tokio::task::JoinError),
    StreamEnd,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Client(err) => write!(f, "{err}"),
            Self::Task(err) => write!(f, "{err}"),
            Self::StreamEnd => write!(f, "Unexpected end of stream"),
        }
    }
}
impl std::error::Error for Error {}
