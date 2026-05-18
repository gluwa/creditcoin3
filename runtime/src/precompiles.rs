//! EVM [`pallet_evm::PrecompileSet`] wiring with delegate-call and caller checks (Frontier /
//! Moonbeam [`precompile_utils::precompile_set`] pattern).

use crate::Runtime;
use pallet_evm_precompile_attestor_stash::AttestorStashPrecompile;
use pallet_evm_precompile_block_prover::BlockProverPrecompile;
use pallet_evm_precompile_bn128::{Bn128Add, Bn128Mul, Bn128Pairing};
use pallet_evm_precompile_chain_info::ChainInfoPrecompile;
use pallet_evm_precompile_ed25519_verifier::Ed25519VerifierPrecompile;
use pallet_evm_precompile_modexp::Modexp;
use pallet_evm_precompile_sha3fips::Sha3FIPS256;
use pallet_evm_precompile_simple::{ECRecover, ECRecoverPublicKey, Identity, Ripemd160, Sha256};
use pallet_evm_precompile_sr25519_verifier::Sr25519VerifierPrecompile;
use pallet_evm_precompile_substrate_transfer::SubstrateTransferPrecompile;

use precompile_utils::precompile_set::*;

/// Standard Ethereum Istanbul precompile addresses: matched with Ethereum (delegatecall allowed).
type EthereumPrecompilesChecks = (AcceptDelegateCall, CallableByContract, CallableByPrecompile);

/// Non-frontier/non-mainnet precompiles: delegatecall *not* allowed (see `common_checks` in precompile-utils).
type NonEthereumPrecompileChecks = (CallableByContract, CallableByPrecompile);

/// Upper bound on the Creditcoin-precompile numeric address band (covers 4049–5050 today with room above).
///
/// Addresses from `AddressU64<1>` through this exclusive band are routed through the tuple below;
/// callers outside `[1, MAX_PRECOMPILE_NUM]` skip the fragment (same pattern as Moonbeam's 1–4095 range).
pub const MAX_PRECOMPILE_NUM: u64 = 8191;

#[precompile_utils::precompile_name_from_address]
type GluwaPrecompilesInner<R> = (
    PrecompileAt<AddressU64<1>, ECRecover, EthereumPrecompilesChecks>,
    PrecompileAt<AddressU64<2>, Sha256, EthereumPrecompilesChecks>,
    PrecompileAt<AddressU64<3>, Ripemd160, EthereumPrecompilesChecks>,
    PrecompileAt<AddressU64<4>, Identity, EthereumPrecompilesChecks>,
    PrecompileAt<AddressU64<5>, Modexp, EthereumPrecompilesChecks>,
    PrecompileAt<AddressU64<6>, Bn128Add, EthereumPrecompilesChecks>,
    PrecompileAt<AddressU64<7>, Bn128Mul, EthereumPrecompilesChecks>,
    PrecompileAt<AddressU64<8>, Bn128Pairing, EthereumPrecompilesChecks>,
    PrecompileAt<AddressU64<1024>, Sha3FIPS256, NonEthereumPrecompileChecks>,
    PrecompileAt<AddressU64<1025>, ECRecoverPublicKey, NonEthereumPrecompileChecks>,
    PrecompileAt<AddressU64<4049>, SubstrateTransferPrecompile<R, ()>, NonEthereumPrecompileChecks>,
    PrecompileAt<AddressU64<4050>, BlockProverPrecompile<R>, NonEthereumPrecompileChecks>,
    PrecompileAt<AddressU64<4051>, ChainInfoPrecompile<R>, NonEthereumPrecompileChecks>,
    PrecompileAt<AddressU64<4052>, AttestorStashPrecompile<R>, NonEthereumPrecompileChecks>,
    PrecompileAt<AddressU64<5049>, Sr25519VerifierPrecompile<R>, NonEthereumPrecompileChecks>,
    PrecompileAt<AddressU64<5050>, Ed25519VerifierPrecompile<R>, NonEthereumPrecompileChecks>,
);

/// Installed [`PrecompileSet`](pallet_evm::PrecompileSet) type for Creditcoin runtime.
pub type GluwaPrecompiles<R> = PrecompileSetBuilder<
    R,
    (
        PrecompilesInRangeInclusive<
            (AddressU64<1>, AddressU64<{ MAX_PRECOMPILE_NUM }>),
            GluwaPrecompilesInner<R>,
        >,
    ),
>;

pub fn used_addresses() -> sp_std::vec::Vec<sp_core::H160> {
    GluwaPrecompiles::<Runtime>::used_addresses_h160().collect()
}
