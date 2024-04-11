use anyhow::Result;
use creditcoin3_attestor_gossip::Felt;
use kameo::{Actor, ActorRef, Message};
use thiserror::Error;
use tracing::{debug, error, info};

use crate::cc3::{self, AttestationSubmit};
use crate::transaction::BlockItem;
use crate::{merkle, merkle::tree::FieldElement, transaction};

#[derive(Debug, Clone)]
pub struct Data {
    pub header_number: u64,
    pub header_hash: [u8; 32],
    pub tx_root: Felt,
    pub rx_root: Felt,
}

impl Data {
    #[must_use]
    pub fn serialize(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        // Serialize header_number as little-endian bytes
        bytes.extend_from_slice(&self.header_number.to_be_bytes().to_vec());

        // Serialize header_hash as little-endian bytes
        bytes.extend_from_slice(&self.header_hash.to_vec());

        // Serialize tx_root as little-endian bytes
        bytes.extend_from_slice(&self.tx_root);

        // Serialize rx_root as little-endian bytes
        bytes.extend_from_slice(&self.rx_root);

        bytes
    }
}

/// Attestor is an actor that creates attestation based on a new block
/// It will pass this attestation to the cc3 client to be submitted on chain
pub struct Attestor {
    pub cc3: ActorRef<cc3::Client>,
}

impl Attestor {
    /// Create a new Attestor given a cc3 client actor
    #[must_use]
    pub fn new(cc3: ActorRef<cc3::Client>) -> Self {
        Self { cc3 }
    }
}

impl Actor for Attestor {}

// Define NewBlock message
pub struct NewBlock {
    pub header_number: u64,
    pub header_hash: [u8; 32],
    pub transactions: Vec<transaction::Transaction>,
    pub receipts: Vec<transaction::Receipt>,
}

impl Message<NewBlock> for Attestor {
    type Reply = Result<()>;

    async fn handle(&mut self, msg: NewBlock) -> Self::Reply {
        // handle the new block
        let attestation = match create(&msg).await {
            Ok(attestation) => attestation,
            Err(e) => {
                error!("Error creating attestation: {:?}", e);
                return Ok(());
            }
        };

        info!("Attestation created succesfully, notifiying cc3 client...");

        // Notify cc3 client with an attestation to be submitted
        let _ = self.cc3.send(AttestationSubmit { attestation }).await?;

        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("No Transactions")]
    NoTransactions,
    #[error("Error building merkle tree")]
    ErrorBuildingMerkleTree,
    #[error("Error unwrapping blockdata {0}")]
    ErrorUnwrappingBlock(String),
}

// Create the attestation data from a ethers::types::Block
// TODO: do all required verification before creating the attestation data
pub async fn create(new_block: &NewBlock) -> Result<Data, Error> {
    // Create rlp's for all transactions
    let mut tx_rlps = new_block
        .transactions
        .iter()
        .map(|tx| tx.to_bytes())
        .collect::<Vec<Vec<u8>>>();

    if tx_rlps.len() == 0 {
        info!("No transactions in block, not doing anything now...");
        return Err(Error::NoTransactions);
    }

    // TODO: see if we can create a tree with 1 element
    // Currently a tree with 1 element gives errors
    if tx_rlps.len() == 1 {
        duplicate_elements(&mut tx_rlps);
    }

    let tx_tree = merkle::tree::create(tx_rlps).map_err(|e| {
        error!("Error creating tree: {:?}", e);
        Error::NoTransactions
    })?;

    let mut rx_rlps = new_block
        .receipts
        .iter()
        .map(|r| r.to_bytes())
        .collect::<Vec<Vec<u8>>>();

    if rx_rlps.len() == 0 {
        info!("No transactions in block, not doing anything now...");
        return Err(Error::NoTransactions);
    }

    // TODO: see if we can create a tree with 1 element
    // Currently a tree with 1 element gives errors
    if rx_rlps.len() == 1 {
        duplicate_elements(&mut rx_rlps);
    }

    let rx_tree = merkle::tree::create(rx_rlps).map_err(|e| {
        error!("Error creating tree: {:?}", e);
        Error::NoTransactions
    })?;

    let attestation = Data {
        header_number: new_block.header_number,
        header_hash: new_block.header_hash,
        tx_root: tx_tree.root().into(),
        rx_root: rx_tree.root().into(),
    };

    debug!("tree tx root: {:?}", attestation.tx_root);
    debug!("tree rx root: {:?}", attestation.rx_root);

    Ok(attestation)
}

fn duplicate_elements<T: Clone>(vec: &mut Vec<T>) {
    let len = vec.len();
    for i in 0..len {
        // Insert a copy of the element at index i immediately after it
        vec.insert(i + 1, vec[i].clone());
    }
}
