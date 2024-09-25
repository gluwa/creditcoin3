#![cfg(feature = "runtime-benchmarks")]

use super::*;
use frame_benchmarking::{account, benchmarks};
use frame_system::RawOrigin;
use pallet_prover_primitives::{Query, VerifierExitStatus};
use sp_std::vec;

benchmarks! {
    submit_proof {
        let who: T::AccountId = account("prover1", 1, 1);

        let query = Query {
            chain_id: 31337,
            height: 1,
            index: 1,
            layout_segments: vec![]
        };
        let query_id = query.id();

        let proof = vec![0; 745676];
    }: _(RawOrigin::Signed(who.clone()), proof, query)
    verify {
        assert_eq!(QueryResultById::<T>::get(query_id), Some(VerifierExitStatus::Success));
    }

    set_stark_program_metadata {
        let who: T::AccountId = account("root", 0, 0);
        let program_auth_hash = 0;
        let program_version = 0;
    }: _(RawOrigin::Root, program_auth_hash, program_version)
}
