use pallet_evm::{
    IsPrecompileResult, Precompile, PrecompileHandle, PrecompileResult, PrecompileSet,
};
use pallet_evm_precompile_substrate_transfer::SubstrateTransferPrecompile;
use sp_core::H160;
use sp_std::marker::PhantomData;

use pallet_evm_precompile_modexp::Modexp;
use pallet_evm_precompile_sha3fips::Sha3FIPS256;
use pallet_evm_precompile_simple::{ECRecover, ECRecoverPublicKey, Identity, Ripemd160, Sha256};

pub struct FrontierPrecompiles<R>(PhantomData<R>);

impl<R> FrontierPrecompiles<R>
where
    R: pallet_evm::Config,
{
    pub fn new() -> Self {
        Self(Default::default())
    }
    pub fn used_addresses() -> [H160; 8] {
        [
            hash(1),    // 0x0000000000000000000000000000000000000001
            hash(2),    // 0x0000000000000000000000000000000000000002
            hash(3),    // 0x0000000000000000000000000000000000000003
            hash(4),    // 0x0000000000000000000000000000000000000004
            hash(5),    // 0x0000000000000000000000000000000000000005
            hash(1024), // 0x0000000000000000000000000000000000000400
            hash(1025), // 0x0000000000000000000000000000000000000401
            hash(4049), // 0x0000000000000000000000000000000000000Fd1
        ]
        // see fn execute() below for an address-->precompile map
    }
}
impl<R> PrecompileSet for FrontierPrecompiles<R>
where
    SubstrateTransferPrecompile<R>: Precompile,
    R: pallet_evm::Config,
{
    fn execute(&self, handle: &mut impl PrecompileHandle) -> Option<PrecompileResult> {
        match handle.code_address() {
            // Ethereum precompiles :
            a if a == hash(1) => Some(ECRecover::execute(handle)),
            a if a == hash(2) => Some(Sha256::execute(handle)),
            a if a == hash(3) => Some(Ripemd160::execute(handle)),
            a if a == hash(4) => Some(Identity::execute(handle)),
            a if a == hash(5) => Some(Modexp::execute(handle)),
            // Non-Frontier specific nor Ethereum precompiles :
            a if a == hash(1024) => Some(Sha3FIPS256::execute(handle)),
            a if a == hash(1025) => Some(ECRecoverPublicKey::execute(handle)),
            a if a == hash(4049) => Some(SubstrateTransferPrecompile::<R, ()>::execute(handle)),
            _ => None,
        }
    }

    fn is_precompile(&self, address: H160, _gas: u64) -> IsPrecompileResult {
        IsPrecompileResult::Answer {
            is_precompile: Self::used_addresses().contains(&address),
            extra_cost: 0,
        }
    }
}

fn hash(a: u64) -> H160 {
    // just converts the argument to its hex value
    H160::from_low_u64_be(a)
}
