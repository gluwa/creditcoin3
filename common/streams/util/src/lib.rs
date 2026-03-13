#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RootInfo {
    pub height: attestor_primitives::Height,
    pub root: attestor_primitives::Digest,
    pub hash: attestor_primitives::Digest,
}

pub type BoxedStream<T> = std::pin::Pin<Box<dyn futures::Stream<Item = T> + Send>>;
