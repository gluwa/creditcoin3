#[derive(Debug)]
pub enum Error {
    Client(eth::Error),
    StreamEnd,
    /// A user-initiated `Interrupt::Stop` (Ctrl+C / service shutdown) propagated out of a block
    /// fetch. This is **not** a connection failure: the outer `StreamRoots` layer recognizes it
    /// and exits cleanly instead of attempting to reconnect.
    Shutdown,
}

impl Error {
    /// True when this error represents a user-initiated graceful shutdown rather than a
    /// recoverable connection failure. Callers must not reconnect on this.
    pub fn is_shutdown(&self) -> bool {
        matches!(self, Self::Shutdown)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Client(err) => write!(f, "{err}"),
            Self::StreamEnd => write!(f, "Unexpected end of stream"),
            Self::Shutdown => write!(f, "shutdown requested"),
        }
    }
}
impl std::error::Error for Error {}
