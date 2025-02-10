// Substrate
use sc_executor::WasmExecutor;
// Local
use creditcoin3_runtime::{opaque::Block, AccountId, Balance, Hash, Nonce};

use crate::eth::EthCompatRuntimeApiCollection;

/// Only enable the benchmarking host functions when we actually want to benchmark.
#[cfg(feature = "runtime-benchmarks")]
pub type HostFunctions = (
    sp_io::SubstrateHostFunctions,
    moonbeam_primitives_ext::moonbeam_ext::HostFunctions,
    proof_verifier::host_benchmark_api::HostFunctions,
    proof_verifier::host_api::HostFunctions,
    frame_benchmarking::benchmarking::HostFunctions,
);
/// Otherwise we only use the default Substrate host functions.
#[cfg(not(feature = "runtime-benchmarks"))]
pub type HostFunctions = (
    sp_io::SubstrateHostFunctions,
    moonbeam_primitives_ext::moonbeam_ext::HostFunctions,
    proof_verifier::host_api::HostFunctions,
);

/// Full backend.
pub type FullBackend = sc_service::TFullBackend<Block>;
/// Full client.
pub type FullClient<RuntimeApi> =
    sc_service::TFullClient<Block, RuntimeApi, WasmExecutor<HostFunctions>>;

pub type Client = FullClient<creditcoin3_runtime::RuntimeApi>;

/// A set of APIs that every runtimes must implement.
pub trait BaseRuntimeApiCollection:
    sp_api::ApiExt<Block>
    + sp_api::Metadata<Block>
    + sp_block_builder::BlockBuilder<Block>
    + sp_offchain::OffchainWorkerApi<Block>
    + sp_session::SessionKeys<Block>
    + sp_transaction_pool::runtime_api::TaggedTransactionQueue<Block>
{
}

impl<Api> BaseRuntimeApiCollection for Api where
    Api: sp_api::ApiExt<Block>
        + sp_api::Metadata<Block>
        + sp_block_builder::BlockBuilder<Block>
        + sp_offchain::OffchainWorkerApi<Block>
        + sp_session::SessionKeys<Block>
        + sp_transaction_pool::runtime_api::TaggedTransactionQueue<Block>
{
}

/// A set of APIs that template runtime must implement.
pub trait RuntimeApiCollection:
    BaseRuntimeApiCollection
    + EthCompatRuntimeApiCollection
    + sp_consensus_babe::BabeApi<Block>
    + sp_consensus_grandpa::GrandpaApi<Block>
    + frame_system_rpc_runtime_api::AccountNonceApi<Block, AccountId, Nonce>
    + pallet_transaction_payment_rpc_runtime_api::TransactionPaymentApi<Block, Balance>
    + attestor_primitives::api::AttestorApi<Block, Hash, AccountId>
    + supported_chains_primitives::api::SupportedChainsApi<Block>
    + randomness_primitives::api::RandomnessPalletApi<Block>
{
}

impl<Api> RuntimeApiCollection for Api where
    Api: BaseRuntimeApiCollection
        + EthCompatRuntimeApiCollection
        + sp_consensus_babe::BabeApi<Block>
        + sp_consensus_grandpa::GrandpaApi<Block>
        + frame_system_rpc_runtime_api::AccountNonceApi<Block, AccountId, Nonce>
        + pallet_transaction_payment_rpc_runtime_api::TransactionPaymentApi<Block, Balance>
        + attestor_primitives::api::AttestorApi<Block, Hash, AccountId>
        + supported_chains_primitives::api::SupportedChainsApi<Block>
        + randomness_primitives::api::RandomnessPalletApi<Block>
{
}
