#[derive(Debug)]
pub enum Error {
    Eth(stream_eth::Error),
    EndOfStream,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}
impl std::error::Error for Error {}
