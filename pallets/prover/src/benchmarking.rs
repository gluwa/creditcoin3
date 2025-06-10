#![cfg(feature = "runtime-benchmarks")]

use super::{Pallet as Prover, *};
use frame_benchmarking::{account, benchmarks};
use frame_system::RawOrigin;
use pallet_prover_primitives::STARK_PROGRAM_V2_HASH;
use sp_core::H256;

benchmarks! {
    set_stark_program_metadata {
        let who: T::AccountId = account("root", 0, 0);
        let program_auth_hash = [0u8; 32];
        let program_version = 0;
    }: _(RawOrigin::Root,  program_version, H256::from_slice(&program_auth_hash))

    remove_stark_program_metadata {
        let who: T::AccountId = account("root", 0, 0);

        let program_version = 2;

        let _ = Prover::<T>::set_stark_program_metadata(RawOrigin::Root.into(), program_version, STARK_PROGRAM_V2_HASH);

    }: _(RawOrigin::Root, program_version)
}
