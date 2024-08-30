use anyhow::Result;
use kameo::{
    actor::Actor,
    message::{Context, Message},
};
use sp_core::H256;
use thiserror::Error;
use tracing::error;

use attestor_primitives::Attestation;
use eth::OrderedBlock;
use mmr::traits::MerkleTreeTrait;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Error building merkle tree")]
    ErrorBuildingMerkleTree,
    #[error("Error building attestation")]
    ErrorBuildingAttestation(String),
    #[error("Error unwrapping blockdata {0}")]
    ErrorUnwrappingBlock(String),
}

/// Attestor is an actor that creates attestation based on a new block
/// It will pass this attestation to the cc3 client to be submitted on chain
#[derive(Debug, Default, Clone)]
pub struct Attestor {}

// Impl kamoe Actor for Attestor
impl Actor for Attestor {}

// Aceept an OrderedBlock as a Message and return an Attestation
impl Message<OrderedBlock> for Attestor {
    /// Reply is the attestation data or error
    type Reply = Result<Option<Attestation<H256>>, Error>;

    async fn handle(
        &mut self,
        msg: OrderedBlock,
        _ctx: Context<'_, Self, Self::Reply>,
    ) -> Self::Reply {
        Ok(Some(create::<H256>(&msg)))
    }
}

// Create the attestation data from a NewBlock
#[must_use]
pub fn create<H>(new_block: &OrderedBlock) -> Attestation<H256> {
    let mt = eth::starknet_pedersen_mmr(new_block);
    Attestation {
        chain_id: new_block.chain_id(),
        header_number: new_block.number(),
        header_hash: sp_core::H256(*new_block.hash().unwrap()),
        root: mt.root().0.to_bytes_be(),
        // We don't have a prev_digest yet, so we set it to None
        prev_digest: None,
    }
}
