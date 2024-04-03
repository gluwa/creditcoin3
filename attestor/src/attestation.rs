use anyhow::Result;
use kameo::{Actor, ActorRef, Message};
use serde::{Deserialize, Serialize};
use tracing::info;
use web3::types::{Block, H256};

use crate::cc3::{self, AttestationSubmit};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationData {
    pub header_number: u64,
    pub header_hash: H256,
}

impl AttestationData {
    pub fn serialize(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        // Serialize header_number as little-endian bytes
        bytes.extend_from_slice(&self.header_number.to_le_bytes());

        // Serialize header_hash as little-endian bytes
        bytes.extend_from_slice(&self.header_hash.as_bytes());

        bytes
    }
}

pub struct Attestor {
    pub cc3: ActorRef<cc3::Client>,
}

impl Attestor {
    pub fn new(cc3: ActorRef<cc3::Client>) -> Self {
        Self { cc3 }
    }
}

impl Actor for Attestor {}

// Define NewBlock message
pub struct NewBlock<T> {
    pub block: Block<T>,
}

impl<B> Message<Attestor> for NewBlock<B>
where
    B: Send + Sync + 'static,
{
    type Reply = Result<()>;

    async fn handle(self, state: &mut Attestor) -> Self::Reply {
        // handle the new block
        let attestation = create_attestation(self.block).await?;

        info!("trying to submit");
        let _ = state.cc3.send(AttestationSubmit { attestation }).await?;

        Ok(())
    }
}

// Create the attestation data from a web3::types::Block
pub async fn create_attestation<T>(block: Block<T>) -> Result<AttestationData> {
    let attestation = AttestationData {
        header_number: block.number.unwrap().as_u64(),
        header_hash: block.hash.unwrap(),
    };

    Ok(attestation)
}
