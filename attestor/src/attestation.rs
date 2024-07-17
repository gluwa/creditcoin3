use anyhow::Result;
use attestor_primitives::{Attestation, ChainId};
use kameo::{
    actor::Actor,
    message::{Context, Message},
};
use mmr::traits::MerkleTreeTrait;
use sp_core::H256;
use thiserror::Error;
use tracing::error;

//use crate::merkle;
use utils::StarknetPedersenMmr;

/// Attestor is an actor that creates attestation based on a new block
/// It will pass this attestation to the cc3 client to be submitted on chain
pub struct Attestor {}

impl Default for Attestor {
    fn default() -> Self {
        Self::new()
    }
}

impl Attestor {
    /// Create a new Attestor given a cc3 client actor
    #[must_use]
    pub fn new() -> Self {
        Self {}
    }
}

impl Actor for Attestor {}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Error building merkle tree")]
    ErrorBuildingMerkleTree,
    #[error("Error building attestation")]
    ErrorBuildingAttestation(String),
    #[error("Error unwrapping blockdata {0}")]
    ErrorUnwrappingBlock(String),
}

// Define NewBlock message
pub struct NewBlock {
    pub chain_id: ChainId,
    pub header_number: u64,
    pub header_hash: H256,
    pub transactions: Vec<eth::transaction::Transaction>,
    pub receipts: Vec<eth::transaction::Receipt>,
}

/// Rlps is the `rlp::encoded` version of either a Transaction or Receipt
pub type Rlps = Vec<Vec<u8>>;

impl NewBlock {
    fn to_transactions_rlps(&self) -> Rlps {
        self.transactions
            .iter()
            .map(eth::transaction::BlockItem::to_bytes)
            .collect::<Vec<Vec<u8>>>()
    }

    fn to_receipts_rlps(&self) -> Rlps {
        self.receipts
            .iter()
            .map(eth::transaction::BlockItem::to_bytes)
            .collect::<Vec<Vec<u8>>>()
    }

    fn get_tx_rx_merkle_trees(&self) -> (StarknetPedersenMmr, StarknetPedersenMmr) {
        // Create rlp's for all transactions
        let tx_rlps = self.to_transactions_rlps();
        let rx_rlps = self.to_receipts_rlps();

        let tx_tree = StarknetPedersenMmr::from(&tx_rlps[..]);
        let rx_tree = StarknetPedersenMmr::from(&rx_rlps[..]);
        // let tx_tree = rlps_to_merkletree(&tx_rlps)?;
        // let rx_tree = rlps_to_merkletree(&rx_rlps)?;

        (tx_tree, rx_tree)
    }
}

impl Message<NewBlock> for Attestor {
    /// Reply is the attestation data or error
    type Reply = Result<Option<Attestation<H256>>, Error>;

    async fn handle(&mut self, msg: NewBlock, _ctx: Context<'_, Self, Self::Reply>) -> Self::Reply {
        // handle the new block
        // let attestation: Option<Attestation<H256>> = match create::<H256>(&msg) {
        //     Ok(attestation) => Some(attestation),
        //     Err(e) => {
        //         warn!("Error creating attestation: {:?}", e);
        //         None
        //     }
        // };

        // Ok(attestation)
        Ok(Some(create::<H256>(&msg)))
    }
}

// Create the attestation data from a NewBlock
// TODO: do all required verification before creating the attestation data
#[must_use] pub fn create<H>(new_block: &NewBlock) -> Attestation<H256> {
    let (tx_tree, rx_tree) = new_block.get_tx_rx_merkle_trees();

    Attestation {
        chain_id: new_block.chain_id,
        header_number: new_block.header_number,
        header_hash: new_block.header_hash,
        tx_root: tx_tree.root().0.to_bytes_be(),
        rx_root: rx_tree.root().0.to_bytes_be(),
        // We don't have a prev_digest yet, so we set it to None
        prev_digest: None,
    }
}

// /// Construct a pedersen merkletree from given input
// fn rlps_to_merkletree(rlps: &Rlps) -> Result<StarknetPedersenMmr, Error> {
//     let tree = merkle::tree::create(rlps).map_err(|e| {
//         error!("Error creating tree: {:?}", e);
//         Error::ErrorBuildingMerkleTree
//     })?;
//     Ok(tree)
// }
