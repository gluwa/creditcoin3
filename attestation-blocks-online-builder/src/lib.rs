//#![feature(trait_alias)]

pub mod attestation_blocks_online_builder;
mod build_attestation_chain_task;
mod check_connectivity;
mod create_attestation_block_task;
mod ethereum_block_listener;
pub mod historical_blocks_provider;
mod network_failures_resilience;
mod purgatory;
pub mod source_blocks_provider;

pub use crate::attestation_blocks_online_builder::*;
pub use crate::historical_blocks_provider::*;
pub use crate::source_blocks_provider::*;
use ethers::providers::Ws;
use futures::future::BoxFuture;
use std::sync::Arc;
use ethereum_types::U256;
//pub use crate::attestation_blocks_online_builder::AttestationChainOnlineBuilder;

type EthersBlock = ethers::types::Block<EthersTxHash>;
type EthersTxHash = ethers::types::TxHash;

// pub trait AsyncCallbackWithArgTrait<Arg, R> = Fn(Arg) -> BoxFuture<'static, R> + Send + Sync + 'static;
// pub trait AsyncCallbackTrait<R> = Fn() -> BoxFuture<'static, R> + Send + Sync + 'static;

// pub type AsyncCallback<R> = Arc<dyn AsyncCallbackTrait<R>>;
// pub type AsyncCallbackWithArg<Arg, R> = Arc<dyn AsyncCallbackWithArgTrait<Arg, R>>;

pub type AsyncCallback<R> = Arc<dyn Fn() -> BoxFuture<'static, R> + Send + Sync + 'static>;
pub type AsyncCallbackWithArg<Arg, R> =
    Arc<dyn Fn(Arg) -> BoxFuture<'static, R> + Send + Sync + 'static>;

pub const SOURCE_BLOCK_TIME_MILLIS: u128 = 12_000;
pub const DEFAULT_MAX_BLOCKS_TO_RETRIEVE: usize = 5;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SourceChainBlockIdentifier(U256);
//pub struct SourceChainBlockIdentifier(u64);

impl SourceChainBlockIdentifier {
    fn block_number(&self) -> U256 {
        self.0
    }
}

impl TryFrom<EthersBlock> for SourceChainBlockIdentifier {
    type Error = ();

    fn try_from(block: EthersBlock) -> Result<Self, Self::Error> {
        Ok(Self(block.number.map(|n| n.as_u64().into()).ok_or(())?))
    }
}

impl From<U256> for SourceChainBlockIdentifier {
    fn from(n: U256) -> Self {
        Self(n)
    }
}

impl From<SourceChainBlockIdentifier> for U256 {
    fn from(b: SourceChainBlockIdentifier) -> Self {
        b.0
    }
}

impl std::fmt::Display for SourceChainBlockIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl SourceChainBlockStream for ethers::providers::SubscriptionStream<'_, Ws, EthersBlock> {
    type SourceBlock = EthersBlock;
}
