use jsonrpsee::{
    core::{async_trait, Error as RpcError, RpcResult},
    proc_macros::rpc,
};
use sp_core::{H160, H256, U256};
use sp_runtime::{traits::Block as BlockT, AccountId32};

use creditcoin3_attestor_gossip::{Attestation, AttestorId, Message, MessageSink, Topic};

#[rpc(client, server)]
pub trait AttestorGossipApi {
    #[method(name = "attestor_submitAttestation")]
    async fn submit_attestation(&self, attestation: AttestationModel) -> RpcResult<()>;
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct EncodedHash(pub sp_core::Bytes);

#[derive(serde::Deserialize, serde::Serialize)]
pub struct AttestationModel {
    pub round: u64,
    pub header_hash: EncodedHash,
    pub header_number: u64,
    pub tx_root: [u8; 32],
    pub rx_root: [u8; 32],
    pub attestor: AttestorIdModel,
    pub topic: TopicModel,
    pub vrf_output: U256,
    pub signature: sp_core::sr25519::Signature,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct AttestorIdModel(pub AccountId32);

#[derive(serde::Deserialize, serde::Serialize)]
pub struct TopicModel(pub u64);

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
impl<B: BlockT> AttestorGossipApiServer for AttestorGossip<B>
where
    <B as BlockT>::Hash: FromBytes,
{
    async fn submit_attestation(&self, attestation: AttestationModel) -> RpcResult<()> {
        let attestation = Attestation {
            header_hash: <B as BlockT>::Hash::from_bytes(attestation.header_hash.0 .0.as_slice())
                .map_err(|e| {
                log::error!("Failed to convert header hash: {:?}", e);
                RpcError::Custom(format!("Failed to convert header hash: {e}"))
            })?,
            header_number: attestation.header_number,
            tx_root: attestation.tx_root,
            rx_root: attestation.rx_root,
            attestor: AttestorId::new(attestation.attestor.0),
            topic: Topic::new(attestation.topic.0),
            round: attestation.round,
            vrf_output: attestation.vrf_output,
            signature: attestation.signature,
        };
        self.sender
            .unbounded_send(Message::Attestation(attestation))
            .unwrap();
        Ok(())
    }
}
