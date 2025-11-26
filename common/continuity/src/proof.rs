use attestor_primitives::block::Block;
use serde::Deserialize;
/// Result of continuity proof generation.
#[derive(Debug, Deserialize)]
pub struct ContinuityProof {
    pub blocks: Vec<Block>,
}

impl ContinuityProof {
    pub fn from_blocks(blocks: Vec<Block>) -> Self {
        Self { blocks }
    }
}
