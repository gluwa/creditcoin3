#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[allow(clippy::unnecessary_cast)]
pub mod weights;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

#[frame_support::pallet]
pub mod pallet {
    use frame_support::{dispatch::DispatchResult, pallet_prelude::*, Blake2_128Concat};
    use frame_system::pallet_prelude::*;
    use prover_primitives::{Query, VerifierExitStatus};
    use sp_core::H256;
    use sp_std::vec::Vec;
    use supported_chains_primitives::provider::SupportedChainsProvider;

    #[cfg(not(feature = "runtime-benchmarks"))]
    use proof_verifier::host_api::verify_proof;
    #[cfg(feature = "runtime-benchmarks")]
    use proof_verifier::host_benchmark_api::verify_proof;

    #[pallet::config]
    pub trait Config: frame_system::Config + pallet_balances::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        type WeightInfo: WeightInfo;
        type SupportedChains: SupportedChainsProvider;
    }

    pub trait WeightInfo {
        fn submit_proof() -> Weight;
    }

    #[pallet::storage]
    #[pallet::getter(fn claim_result_by_hash)]
    pub type QueryResultById<T: Config> = StorageMap<
        Hasher = Blake2_128Concat,
        Key = H256,
        Value = VerifierExitStatus,
        QueryKind = OptionQuery,
    >;

    #[pallet::pallet]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        QueryReceived(H256, T::AccountId),
        QueryVerified(H256, T::AccountId, VerifierExitStatus),
    }

    #[pallet::error]
    pub enum Error<T> {
        InvalidProofSubmitted,
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::call_index(0)]
        #[pallet::weight(<T as Config>::WeightInfo::submit_proof())]
        pub fn submit_proof(origin: OriginFor<T>, proof: Vec<u8>, query: Query) -> DispatchResult {
            let prover = ensure_signed(origin)?;

            Self::deposit_event(Event::<T>::QueryReceived(query.id(), prover.clone()));

            // Pre eliminary check
            ensure!(!proof.is_empty(), Error::<T>::InvalidProofSubmitted);

            // Verify proof
            let result = verify_proof(proof);
            ensure!(result, Error::<T>::InvalidProofSubmitted);

            let query_id = query.id();

            // Deposit event
            Self::deposit_event(Event::<T>::QueryVerified(
                query_id,
                prover,
                VerifierExitStatus::Success,
            ));

            QueryResultById::<T>::insert(query_id, VerifierExitStatus::Success);

            Ok(())
        }
    }
}
