use crate::merkle::tree::BinaryMerkle;
use crate::{merkle, transaction};
use anyhow::Result;
use attestor_primitives::AttestationData;
use kameo::{Actor, Message};
use sp_core::H256;
use thiserror::Error;
use tracing::{debug, error, info};

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
    #[error("No Transactions")]
    NoTransactions,
    #[error("Error building merkle tree")]
    ErrorBuildingMerkleTree,
    #[error("Error building attestation")]
    ErrorBuildingAttestation(String),
    #[error("Error unwrapping blockdata {0}")]
    ErrorUnwrappingBlock(String),
}

// Define NewBlock message
pub struct NewBlock {
    pub header_number: u64,
    pub header_hash: H256,
    pub transactions: Vec<transaction::Transaction>,
    pub receipts: Vec<transaction::Receipt>,
}

/// Rlps is the `rlp::encoded` version of either a Transaction or Receipt
pub type Rlps = Vec<Vec<u8>>;

impl NewBlock {
    fn to_transactions_rlps(&self) -> Rlps {
        self.transactions
            .iter()
            .map(transaction::BlockItem::to_bytes)
            .collect::<Vec<Vec<u8>>>()
    }

    fn to_receipts_rlps(&self) -> Rlps {
        self.receipts
            .iter()
            .map(transaction::BlockItem::to_bytes)
            .collect::<Vec<Vec<u8>>>()
    }

    fn get_tx_rx_merkle_trees(&self) -> Result<(BinaryMerkle, BinaryMerkle), Error> {
        // Create rlp's for all transactions
        let tx_rlps = self.to_transactions_rlps();
        let rx_rlps = self.to_receipts_rlps();

        let tx_tree = rlps_to_merkletree(tx_rlps)?;
        let rx_tree = rlps_to_merkletree(rx_rlps)?;

        Ok((tx_tree, rx_tree))
    }
}

impl Message<NewBlock> for Attestor {
    /// Reply is the attestation data or error
    type Reply = Result<AttestationData, Error>;

    async fn handle(&mut self, msg: NewBlock) -> Self::Reply {
        // handle the new block
        let attestation = match create(&msg) {
            Ok(attestation) => attestation,
            Err(e) => {
                error!("Error creating attestation: {:?}", e);
                return Err(Error::ErrorBuildingAttestation(e.to_string()));
            }
        };

        Ok(attestation)
    }
}

// Create the attestation data from a NewBlock
// TODO: do all required verification before creating the attestation data
pub fn create(new_block: &NewBlock) -> Result<AttestationData, Error> {
    let (tx_tree, rx_tree) = new_block.get_tx_rx_merkle_trees()?;

    let attestation = AttestationData {
        header_number: new_block.header_number,
        header_hash: new_block.header_hash,
        tx_root: tx_tree.root().into(),
        rx_root: rx_tree.root().into(),
    };

    debug!("tree tx root: {:?}", attestation.tx_root);
    debug!("tree rx root: {:?}", attestation.rx_root);

    Ok(attestation)
}

/// Construct a pedersen merkletree from given input
fn rlps_to_merkletree(mut rlps: Rlps) -> Result<merkle::tree::BinaryMerkle, Error> {
    if rlps.is_empty() {
        info!("No transactions in block, not doing anything now...");
        return Err(Error::NoTransactions);
    }

    // TODO: see if we can create a tree with 1 element
    // Currently a tree with 1 element gives errors
    if rlps.len() == 1 {
        duplicate_elements(&mut rlps);
    }

    let tree = merkle::tree::create(rlps).map_err(|e| {
        error!("Error creating tree: {:?}", e);
        Error::NoTransactions
    })?;

    Ok(tree)
}

fn duplicate_elements<T: Clone>(vec: &mut Vec<T>) {
    let len = vec.len();
    for i in 0..len {
        // Insert a copy of the element at index i immediately after it
        vec.insert(i + 1, vec[i].clone());
    }
}
