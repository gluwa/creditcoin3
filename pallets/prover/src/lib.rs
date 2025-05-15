#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[allow(clippy::unnecessary_cast)]
pub mod weights;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

mod benchmarking;
pub mod test_helpers;

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
        QueryVerificationFailed(H256, T::AccountId, VerifierExitStatus),
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
        MissingContinuityProof,
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
                let (status, result_segments, continuity_proof_len, continuity_checkpoint_digest) =
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

                        let continuity_proof_len = if let Some(len) = continuity_proof_len {
                            len
                        } else {
                            Self::deposit_event(Event::<T>::QueryVerificationFailed(
                                query_id,
                                prover.clone(),
                                VerifierExitStatus::MissingContinuityProof,
                            ));
                            QueryResultById::<T>::insert(
                                query_id,
                                VerifierExitStatus::MissingContinuityProof,
                            );
                            return Err(Error::<T>::MissingContinuityProof.into());
                        };

                        let continuity_checkpoint_digest =
                            if let Some(digest) = continuity_checkpoint_digest {
                                digest
                            } else {
                                Self::deposit_event(Event::<T>::QueryVerificationFailed(
                                    query_id,
                                    prover.clone(),
                                    VerifierExitStatus::MissingContinuityProof,
                                ));
                                QueryResultById::<T>::insert(
                                    query_id,
                                    VerifierExitStatus::MissingContinuityProof,
                                );
                                return Err(Error::<T>::MissingContinuityProof.into());
                            };

                        let checkpoint_block_number = query.height - 1 + continuity_proof_len - 1;

                        let expected_block_number = if let Some(last_checkpoint_number) =
                            T::Checkpoints::get_last_checkpoint_number(query.chain_id)
                        {
                            // Use a checkpoint if one is available
                            if last_checkpoint_number >= query.height {
                                // Get the checkpoint using our digest from the proof. If this fails, then
                                // the proof is invalid.
                                let checkpoint = T::Checkpoints::get_checkpoint(
                                    query.chain_id,
                                    continuity_checkpoint_digest,
                                );
                                if let Some(check) = checkpoint {
                                    check
                                } else {
                                    log::error!(
                                        "No checkpoint for digest: {:?}",
                                        continuity_checkpoint_digest
                                    );
                                    return Err(Error::<T>::QueryCheckpointMismatch.into());
                                }
                            } else {
                                let attestation = T::Attestations::get_attestation(
                                    query.chain_id,
                                    continuity_checkpoint_digest,
                                );
                                if let Some(att) = attestation {
                                    att.header_number()
                                } else {
                                    return Err(Error::<T>::QueryCheckpointMismatch.into());
                                }
                            }
                        } else {
                            let attestation = T::Attestations::get_attestation(
                                query.chain_id,
                                continuity_checkpoint_digest,
                            );
                            if let Some(att) = attestation {
                                att.header_number()
                            } else {
                                return Err(Error::<T>::QueryCheckpointMismatch.into());
                            }
                        };

                        if checkpoint_block_number != expected_block_number {
                            Self::deposit_event(Event::<T>::QueryVerificationFailed(
                                query_id,
                                prover.clone(),
                                VerifierExitStatus::QueryBlockNumberMismatch,
                            ));
                            QueryResultById::<T>::insert(
                                query_id,
                                VerifierExitStatus::QueryBlockNumberMismatch,
                            );
                            return Err(Error::<T>::QueryBlockNumberMismatch.into());
                        }
                        ()
                    }
                    1..=5 => {
                        Self::deposit_event(Event::<T>::QueryVerificationFailed(
                            query_id,
                            prover.clone(),
                            VerifierExitStatus::ProcessingError,
                        ));
                        QueryResultById::<T>::insert(query_id, VerifierExitStatus::ProcessingError);
                        return Err(Error::<T>::FileError.into());
                    }
                    6 | 7 => {
                        Self::deposit_event(Event::<T>::QueryVerificationFailed(
                            query_id,
                            prover.clone(),
                            VerifierExitStatus::ProcessingError,
                        ));
                        QueryResultById::<T>::insert(query_id, VerifierExitStatus::ProcessingError);
                        return Err(Error::<T>::ProofParseError.into());
                    }
                    8 => {
                        Self::deposit_event(Event::<T>::QueryVerificationFailed(
                            query_id,
                            prover.clone(),
                            VerifierExitStatus::ProcessingError,
                        ));
                        QueryResultById::<T>::insert(query_id, VerifierExitStatus::ProcessingError);
                        return Err(Error::<T>::StarkProgramAuthenticationError.into());
                    }
                    9 => {
                        Self::deposit_event(Event::<T>::QueryVerificationFailed(
                            query_id,
                            prover.clone(),
                            VerifierExitStatus::ProcessingError,
                        ));
                        QueryResultById::<T>::insert(query_id, VerifierExitStatus::ProcessingError);
                        return Err(Error::<T>::VerifierExecutionError.into());
                    }
                    10 => {
                        Self::deposit_event(Event::<T>::QueryVerificationFailed(
                            query_id,
                            prover.clone(),
                            VerifierExitStatus::ProcessingError,
                        ));
                        QueryResultById::<T>::insert(query_id, VerifierExitStatus::ProcessingError);
                        return Err(Error::<T>::VerifierProcessError.into());
                    }
                    11 => {
                        Self::deposit_event(Event::<T>::QueryVerificationFailed(
                            query_id,
                            prover,
                            VerifierExitStatus::QueryValidationError,
                        ));
                        QueryResultById::<T>::insert(
                            query_id,
                            VerifierExitStatus::QueryValidationError,
                        );
                        return Err(Error::<T>::QueryIdNotValidated.into());
                    }
                    12 => {
                        Self::deposit_event(Event::<T>::QueryVerificationFailed(
                            query_id,
                            prover,
                            VerifierExitStatus::QueryOutOfBounds,
                        ));

                        QueryResultById::<T>::insert(
                            query_id,
                            VerifierExitStatus::QueryOutOfBounds,
                        );
                        return Err(Error::<T>::QueryOutOfBounds.into());
                    }
                    13 => {
                        Self::deposit_event(Event::<T>::QueryVerificationFailed(
                            query_id,
                            prover,
                            VerifierExitStatus::LayoutMismatch,
                        ));

                        QueryResultById::<T>::insert(query_id, VerifierExitStatus::LayoutMismatch);

                        return Err(Error::<T>::QueryOffsetMismatch.into());
                    }
                    _ => {
                        Self::deposit_event(Event::<T>::QueryVerificationFailed(
                            query_id,
                            prover,
                            VerifierExitStatus::ProofInvalid,
                        ));

                        QueryResultById::<T>::insert(query_id, VerifierExitStatus::ProofInvalid);

                        return Err(Error::<T>::InvalidProofSubmitted.into());
                    }
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
