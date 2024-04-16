use jsonrpsee::{
    core::{async_trait, RpcResult},
    proc_macros::rpc,
};
use serde::Serialize;
use sp_core::{H160, H256};
use sp_runtime::traits::Block as BlockT;

use creditcoin3_attestor_gossip::{Attestation, Message, MessageSink};

#[rpc(client, server)]
pub trait AttestorGossipApi<H>
where
    H: Serialize,
{
    #[method(name = "attestor_submitAttestation")]
    async fn submit_attestation(&self, attestation: Attestation<H>) -> RpcResult<()>;
}

pub struct AttestorGossip<B: BlockT> {
    sender: MessageSink<B>,
}

impl<B: BlockT> AttestorGossip<B> {
    pub fn new(sender: MessageSink<B>) -> Self {
        Self { sender }
    }
}

trait FromBytes: Sized {
    type Error: std::error::Error;
    fn from_bytes(bytes: &[u8]) -> Result<Self, Self::Error>;
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("Incorrect length: expected {0} bytes, got {1}")]
    IncorrectLength(usize, usize),
}

macro_rules! impl_from_bytes_hash {
    (for $($hash_ty: ident ($len: literal)),+) => {
        $(
            impl FromBytes for $hash_ty {
                type Error = Error;
                fn from_bytes(bytes: &[u8]) -> Result<Self, Self::Error> {
                    if (bytes.len() != $len) {
                        return Err(Error::IncorrectLength($len, bytes.len()));
                    }
                    Ok($hash_ty::from_slice(bytes))
                }
            }
        )+
    };
}

impl_from_bytes_hash!(for H256(32), H160(20));

#[async_trait]
impl<B> AttestorGossipApiServer<<B as BlockT>::Hash> for AttestorGossip<B>
where
    B: BlockT,
    <B as BlockT>::Hash: FromBytes,
{
    async fn submit_attestation(
        &self,
        attestation: Attestation<<B as BlockT>::Hash>,
    ) -> RpcResult<()> {
        self.sender
            .unbounded_send(Message::Attestation(attestation))
            .unwrap();
        Ok(())
    }
}
