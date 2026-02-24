/// Generic block trait, used to describe any entity that can be treated as a block, such as an Ethereum block or a Substrate block.
/// This is used to abstract over different block types and allow for chain-agnostic implementations of block related functionality.
pub trait BlockLike {
    type BlockNumber;
    type BlockHash;
    type TxRx;

    fn number(&self) -> Self::BlockNumber;
    fn hash(&self) -> Self::BlockHash;
    fn transactions(&self) -> &[Self::TxRx];
}

/// A trait for a block sink, which is responsible for consuming blocks and processing them in some way.
/// This could be used for storing blocks, indexing them, or any other kind of processing.
/// The block sink should be able to handle blocks in order and keep track of the next needed block height.
pub trait BlockSink {
    type Block: BlockLike;

    /// Pushes a series of blocks to the sink.
    /// The sink should not expected anything about the order of the blocks, but should be able to handle them in order based on their block numbers.
    fn push(&mut self, blocks: impl IntoIterator<Item = Self::Block>);

    /// Returns the next block height that the sink needs to process.
    fn next_needed_height(&self) -> <Self::Block as BlockLike>::BlockNumber;
}

#[cfg(feature = "threaded_sink")]
mod threaded_sink {
    use super::*;

    pub type SharedSink<T> = std::sync::Arc<parking_lot::Mutex<T>>;

    impl<T> BlockSink for SharedSink<T>
    where
        T: BlockSink,
    {
        type Block = T::Block;

        fn push(&mut self, blocks: impl IntoIterator<Item = Self::Block>) {
            let mut sink = self.lock();
            sink.push(blocks);
        }

        fn next_needed_height(&self) -> <Self::Block as BlockLike>::BlockNumber {
            let sink = self.lock();
            sink.next_needed_height()
        }
    }
}

#[cfg(feature = "threaded_sink")]
pub use threaded_sink::*;
