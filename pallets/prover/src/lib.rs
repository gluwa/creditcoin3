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
    use pallet_prover_primitives::{Query, VerifierExitStatus};
    use sp_core::H256;
    use sp_std::prelude::*;
    use supported_chains_primitives::provider::SupportedChainsProvider;

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

    #[pallet::storage]
    #[pallet::getter(fn stark_program_metadata)]
    pub type StarkProgramMetadata<T: Config> =
        StorageMap<Hasher = Blake2_128Concat, Key = u8, Value = u64, QueryKind = ValueQuery>;

    #[pallet::storage]
    #[pallet::getter(fn last_version)]
    pub type LastVersion<T: Config> = StorageValue<_, u8, ValueQuery>;

    #[pallet::pallet]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        QueryVerified(H256, T::AccountId, VerifierExitStatus),

        MetadataSet(u8, u64),
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

            // Pre eliminary check
            ensure!(!proof.is_empty(), Error::<T>::InvalidProofSubmitted);

            #[cfg(not(test))]
            let metadata = StarkProgramMetadata::<T>::iter().collect::<Vec<(u8, u64)>>();

            #[cfg(not(test))]
            let last_version = LastVersion::<T>::get();

            // have to do this weird thing for now to use different functions for tests,
            // benchmarks and regular execution
            // ToDo: remove fn verify_proof from mock and benchmarks and reuse the correct fn in all places

            // Verify proof during regular execution
            #[cfg(all(not(feature = "runtime-benchmarks"), not(test)))]
            let result = proof_verifier::host_api::verify_proof(
                proof,
                query.clone(),
                metadata,
                last_version,
            );

            // Verify proof for runtime benchmarks
            #[cfg(feature = "runtime-benchmarks")]
            let result = proof_verifier::host_benchmark_api::verify_proof(
                proof,
                query.clone(),
                metadata,
                last_version,
            );

            // Verify proof for tests
            #[cfg(test)]
            let result = crate::mock::verify_proof(proof);

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

        #[pallet::call_index(1)]
        #[pallet::weight({10_000})]
        pub fn set_stark_program_metadata(
            origin: OriginFor<T>,
            program_auth_hash: u64,
            program_version: u8,
        ) -> DispatchResult {
            ensure_root(origin)?;

            ensure!(
                !StarkProgramMetadata::<T>::contains_key(program_version),
                "Program version already exists"
            );

            StarkProgramMetadata::<T>::insert(program_version, program_auth_hash);

            LastVersion::<T>::put(program_version);

            Self::deposit_event(Event::<T>::MetadataSet(program_version, program_auth_hash));

            Ok(())
        }
    }
}
