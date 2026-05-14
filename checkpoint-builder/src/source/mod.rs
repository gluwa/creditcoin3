//! Source module for reading block root data.

pub mod archive;
pub mod sled;

pub use self::archive::ArchiveSource;
pub use self::sled::SledSource;

use anyhow::Result;
use attestor_primitives::Digest;

/// Information about a single block root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootInfo {
    pub digest: Digest,
    pub height: u64,
}

/// Trait for sources that provide block root data.
pub trait RootSource {
    fn get(&self, height: u64) -> Result<Option<RootInfo>>;
    fn get_range(&self, start_height: u64, end_height: u64) -> Result<Vec<RootInfo>>;
    fn first(&self) -> Result<Option<RootInfo>>;
    fn last(&self) -> Result<Option<RootInfo>>;
    fn iter_range(
        &self,
        start_height: u64,
        end_height: u64,
    ) -> Box<dyn Iterator<Item = Result<RootInfo>> + '_>;
}
