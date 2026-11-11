//! Weights for code that lives outside this repository's pallets, generated
//! from the runtime's own benchmarks with `.github/ethereum-weights.hbs` by
//! the BENCHMARKS workflow (the `pallet-ethereum-weights` artifact).

use crate::Runtime;
use frame_support::weights::Weight;

/// Whole EVM blocks, executed and finalized. Only the production-pricing
/// tests read them.
#[cfg(all(test, not(any(feature = "devnet", feature = "fast-runtime"))))]
pub mod evm_block;
/// `pallet_ethereum`'s `on_finalize`, from `ethereum_finalize_benchmarking`.
#[allow(dead_code)]
pub mod pallet_ethereum;

/// Reserves `on_finalize`'s fixed cost, measured on a block without Ethereum
/// transactions. Its per-transaction cost is paid through gas, which
/// `evm_block_benchmarking`'s tests check `WeightPerGas` covers.
pub struct EthereumWeightInfo;

impl ::pallet_ethereum::weights::WeightInfo for EthereumWeightInfo {
    fn on_finalize_base() -> Weight {
        pallet_ethereum::WeightInfo::<Runtime>::on_finalize_transactions(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frame_support::traits::Hooks;

    #[test]
    fn ethereum_on_initialize_reserves_the_benchmarked_finalize_base() {
        sp_io::TestExternalities::default().execute_with(|| {
            let base =
                <EthereumWeightInfo as ::pallet_ethereum::weights::WeightInfo>::on_finalize_base();
            let reserved = crate::Ethereum::on_initialize(1);
            assert!(
                reserved.all_gte(base),
                "{reserved:?} reserves less than {base:?}"
            );
        });
    }
}
