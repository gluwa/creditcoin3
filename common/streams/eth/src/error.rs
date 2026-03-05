#[derive(Debug)]
pub enum Error {
    Client(eth::Error),
    StreamEnd,
    Task(tokio::task::JoinError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}
impl std::error::Error for Error {}
