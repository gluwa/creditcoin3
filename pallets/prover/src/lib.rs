#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[allow(clippy::unnecessary_cast)]
pub mod weights;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

mod benchmarking;

#[frame_support::pallet]
pub mod pallet {
    use attestor_primitives::provider::{AttestationProvider, CheckpointProvider};
    use frame_support::{dispatch::DispatchResult, pallet_prelude::*, Blake2_128Concat};
    use frame_system::pallet_prelude::*;
    use pallet_prover_primitives::{
        Query, ResultSegment, VerifierExitStatus, STARK_PROGRAM_V1_HASH, STARK_PROGRAM_V2_HASH,
        STARK_PROGRAM_V3_HASH,
    };
    use sp_core::H256;
    use sp_std::prelude::*;
    use supported_chains_primitives::provider::SupportedChainsProvider;

    #[pallet::config]
    pub trait Config: frame_system::Config + pallet_balances::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        type WeightInfo: WeightInfo;
        type SupportedChains: SupportedChainsProvider;
        type Checkpoints: CheckpointProvider;
        type Attestations: AttestationProvider<Self::Hash, Self::AccountId>;
        #[pallet::constant]
        type MaxSegmentsPerVerifierResult: Get<u32>;
    }

    pub trait WeightInfo {
        fn submit_proof() -> Weight;
        fn set_stark_program_metadata() -> Weight;
        fn remove_stark_program_metadata() -> Weight;
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
    #[pallet::getter(fn result_segments_by_id)]
    pub type ResultSegmentsById<T: Config> = StorageMap<
        Hasher = Blake2_128Concat,
        Key = H256,
        Value = BoundedVec<ResultSegment, T::MaxSegmentsPerVerifierResult>,
        QueryKind = OptionQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn stark_program_metadata)]
    pub type StarkProgramMetadata<T: Config> =
        StorageMap<Hasher = Blake2_128Concat, Key = u8, Value = H256, QueryKind = ValueQuery>;

    #[pallet::pallet]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    #[pallet::genesis_config]
    #[derive(frame_support::DefaultNoBound)]
    pub struct GenesisConfig<T> {
        pub _phantom: PhantomData<T>,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            StarkProgramMetadata::<T>::insert(1, STARK_PROGRAM_V1_HASH);
            StarkProgramMetadata::<T>::insert(2, STARK_PROGRAM_V2_HASH);
            StarkProgramMetadata::<T>::insert(3, STARK_PROGRAM_V3_HASH);
        }
    }

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        QueryVerified(H256, T::AccountId, VerifierExitStatus),
        StarkProgramMetadataSet(u8, H256),
        StarkProgramMetadataRemoved(u8),
    }

    #[pallet::error]
    pub enum Error<T> {
        InvalidProofSubmitted,
        StarkProgramMetadataNotSet,
        StarkProgramMetadataAlreadySet,
        StarkProgramMetadataNotFound,
        FileError,
        ProofParseError,
        StarkProgramAuthenticationError,
        VerifierExecutionError,
        VerifierProcessError,
        QueryIdNotValidated,
        QueryOutOfBounds,
        QueryOffsetMismatch,
        QueryCheckpointMismatch,
        QueryBlockNumberMismatch,
        ResultSegmentsExceedMaxSize,
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::call_index(0)]
        #[pallet::weight(<T as Config>::WeightInfo::submit_proof())]
        pub fn submit_proof(origin: OriginFor<T>, proof: Vec<u8>, query: Query) -> DispatchResult {
            let prover = ensure_signed(origin)?;

            // Pre eliminary check
            ensure!(!proof.is_empty(), Error::<T>::InvalidProofSubmitted);

            let metadata = StarkProgramMetadata::<T>::iter().collect::<Vec<_>>();

            ensure!(!metadata.is_empty(), Error::<T>::StarkProgramMetadataNotSet);

            let query_id = query.id();

            #[cfg(not(feature = "runtime-benchmarks"))]
            {
                let (status, result_segments, continuity_proof_len) =
                    proof_verifier::host_api::verify_proof(proof, query.clone(), metadata);

                match status {
                    0 => {
                        // the continuity chain contains blocks from query.height - 1 to the checkpoint/attestation included,
                        // this means that the `continuity_proof_len` includes both of the borders, so we need to subtract 1
                        // to get to the starting point, which is query.height - 1, and then subtract additional 1 not to
                        // count both of the borders.
                        // example: query.height = 6, checkpoint/attestation = 10,
                        // continuity_proof_len = len(5..10) = 6 (both borders included),
                        // checkpoint number generated from the proof = 6 - 1 + 6 - 1 = 10
                        let checkpoint_block_number =
                            query.height - 1 + continuity_proof_len.unwrap() - 1;

                        let checkpoint_interval =
                            T::Checkpoints::get_checkpoint_interval(query.chain_id);

                        let attestation_interval =
                            T::Attestations::get_attestation_interval(query.chain_id);

                        let expected_block_number = if let Some(last_checkpoint_number) =
                            T::Checkpoints::get_last_checkpoint_number(query.chain_id)
                        {
                            if last_checkpoint_number
                                > query.height + checkpoint_interval * attestation_interval
                            {
                                // number of blocks per checkpoint = checkpoint_interval * attestation_interval
                                // N of the checkpoint that will include the query =
                                // ceil(query.height / (checkpoint_interval * attestation_interval))
                                checkpoint_interval
                                    * attestation_interval
                                    * (query.height / (checkpoint_interval * attestation_interval)
                                        + (query.height
                                            % (checkpoint_interval * attestation_interval)
                                            != 0) as u64)
                            } else {
                                // if we cant rely on a checkpoint, use the same formula as above
                                // but with attestation interval which is expressed in blocks
                                attestation_interval
                                    * (query.height / attestation_interval
                                        + (query.height % attestation_interval != 0) as u64)
                            }
                        } else {
                            attestation_interval
                                * (query.height / attestation_interval
                                    + (query.height % attestation_interval != 0) as u64)
                        };

                        ensure!(
                            checkpoint_block_number == expected_block_number,
                            Error::<T>::QueryBlockNumberMismatch
                        );
                        ()
                    }
                    1..=5 => return Err(Error::<T>::FileError.into()),
                    6 | 7 => return Err(Error::<T>::ProofParseError.into()),
                    8 => return Err(Error::<T>::StarkProgramAuthenticationError.into()),
                    9 => return Err(Error::<T>::VerifierExecutionError.into()),
                    10 => return Err(Error::<T>::VerifierProcessError.into()),
                    11 => return Err(Error::<T>::QueryIdNotValidated.into()),
                    12 => return Err(Error::<T>::QueryOutOfBounds.into()),
                    13 => return Err(Error::<T>::QueryOffsetMismatch.into()),
                    _ => return Err(Error::<T>::InvalidProofSubmitted.into()),
                }

                #[cfg(not(feature = "runtime-benchmarks"))]
                {
                    let bounded_segments: BoundedVec<
                        ResultSegment,
                        <T as Config>::MaxSegmentsPerVerifierResult,
                    > = frame_support::BoundedVec::try_from(result_segments)
                        .map_err(|_| Error::<T>::ResultSegmentsExceedMaxSize)?;
                    ResultSegmentsById::<T>::insert(query_id, bounded_segments);
                }
            }

            #[cfg(feature = "runtime-benchmarks")]
            let result =
                proof_verifier::host_benchmark_api::verify_proof(proof, query.clone(), metadata);

            #[cfg(feature = "runtime-benchmarks")]
            ensure!(result, Error::<T>::InvalidProofSubmitted);

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
        #[pallet::weight(<T as Config>::WeightInfo::set_stark_program_metadata())]
        pub fn set_stark_program_metadata(
            origin: OriginFor<T>,
            program_version: u8,
            program_auth_hash: H256,
        ) -> DispatchResult {
            ensure_root(origin)?;

            ensure!(
                !StarkProgramMetadata::<T>::contains_key(program_version),
                Error::<T>::StarkProgramMetadataAlreadySet
            );

            // Insert the metadata
            StarkProgramMetadata::<T>::insert(program_version, program_auth_hash);

            Self::deposit_event(Event::<T>::StarkProgramMetadataSet(
                program_version,
                program_auth_hash,
            ));

            Ok(())
        }

        // Remove metadata
        #[pallet::call_index(2)]
        #[pallet::weight(<T as Config>::WeightInfo::remove_stark_program_metadata())]
        pub fn remove_stark_program_metadata(
            origin: OriginFor<T>,
            program_version: u8,
        ) -> DispatchResult {
            ensure_root(origin)?;

            ensure!(
                StarkProgramMetadata::<T>::contains_key(program_version),
                Error::<T>::StarkProgramMetadataNotFound
            );

            // Remove the metadata
            StarkProgramMetadata::<T>::remove(program_version);

            Self::deposit_event(Event::<T>::StarkProgramMetadataRemoved(program_version));

            Ok(())
        }
    }
}
