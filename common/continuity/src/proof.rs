use attestor_primitives::block::Block;

/// Result of continuity proof generation.
#[derive(Debug)]
pub struct ContinuityProof {
    pub blocks: Vec<Block>,
}

impl ContinuityProof {
    pub fn from_blocks(blocks: Vec<Block>) -> Self {
        Self { blocks }
    }
}
