#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RootInfo {
    pub height: attestor_primitives::Height,
    pub root: attestor_primitives::Digest,
    pub hash: attestor_primitives::Digest,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttestationInfo {
    pub digest: attestor_primitives::Digest,
    pub height: attestor_primitives::Height,
}

pub type BoxedStream<T> = std::pin::Pin<Box<dyn futures::Stream<Item = T> + Send>>;
