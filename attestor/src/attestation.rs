use anyhow::Result;
use kameo::{Actor, ActorRef, Message};
use tracing::info;
use web3::types::{Block, H256};

use creditcoin3_attestor_gossip::{Attestation, AttestorId, Topic};

use crate::cc3::{self, AttestationSubmit, GetAttestorId};

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
        let attestor_id = state.cc3.send(GetAttestorId {}).await??;
        // handle the new block
        let attestation = create_attestation(self.block, attestor_id).await?;

        info!("trying to submit");
        let _ = state.cc3.send(AttestationSubmit { attestation }).await?;

        Ok(())
    }
}

pub async fn create_attestation<T>(
    block: Block<T>,
    attestor_id: AttestorId,
) -> Result<Attestation<H256>> {
    let attestation = Attestation {
        round: 1,
        header_number: block.number.unwrap().as_u64(),
        attestor: attestor_id.clone(),
        header_hash: block.hash.unwrap(),
        topic: Topic::new(1),
    };

    Ok(attestation)
}
