use alloy::primitives::{B256, U256};
use alloy::rpc::types::eth::{Block, BlockTransactions};
use anyhow::Result;
use kameo::{Actor, ActorRef, Message};
use starknet_crypto::FieldElement;
use thiserror::Error;
use tracing::{debug, error, info};

use crate::cc3::{self, AttestationSubmit};
use crate::merkle;
use crate::transaction::BlockItem;

#[derive(Debug, Clone)]
pub struct Data {
    pub header_number: U256,
    pub header_hash: B256,
    pub tx_root: FieldElement,
    pub rx_root: FieldElement,
}

impl Data {
    #[must_use]
    pub fn serialize(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        // Serialize header_number as little-endian bytes
        bytes.extend_from_slice(&self.header_number.to_be_bytes_vec());

        // Serialize header_hash as little-endian bytes
        bytes.extend_from_slice(&self.header_hash.to_vec());

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
    pub block: Block,
}

impl Message<NewBlock> for Attestor {
    type Reply = Result<()>;

    async fn handle(&mut self, msg: NewBlock) -> Self::Reply {
        // handle the new block
        let attestation = match create(&msg.block).await {
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
pub async fn create(block: &Block) -> Result<Data, Error> {
    // Create rlp's for all transactions
    let mut rlps = match &block.transactions {
        BlockTransactions::Full(tx) => tx
            .into_iter()
            .map(|tx| super::transaction::Transaction(tx.clone()).to_bytes())
            .collect(),
        _ => {
            info!("No full tx");
            vec![]
        }
    };

    if rlps.len() == 0 {
        info!("No transactions in block, not doing anything now...");
        return Err(Error::NoTransactions);
    }

    // TODO: see if we can create a tree with 1 element
    // Currently a tree with 1 element gives errors
    if rlps.len() == 1 {
        duplicate_elements(&mut rlps);
    }

    let tx_tree = merkle::tree::create(rlps).map_err(|e| {
        error!("Error creating tree: {:?}", e);
        Error::NoTransactions
    })?;

    let attestation = Data {
        header_number: U256::saturating_from(
            block
                .header
                .number
                .ok_or(Error::ErrorUnwrappingBlock("Block number".to_string()))?,
        ),
        header_hash: block
            .header
            .hash
            .ok_or(Error::ErrorUnwrappingBlock("Block hash".to_string()))?,
        tx_root: tx_tree.root().into(),
        rx_root: tx_tree.root().into(),
    };

    debug!("tree tx root: {:?}", attestation.tx_root);

    Ok(attestation)
}

fn duplicate_elements<T: Clone>(vec: &mut Vec<T>) {
    let len = vec.len();
    for i in 0..len {
        // Insert a copy of the element at index i immediately after it
        vec.insert(i + 1, vec[i].clone());
    }
}
