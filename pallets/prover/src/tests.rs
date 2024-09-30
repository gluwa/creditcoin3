use self::mock::PROVER_3;

use super::*;
use pallet_prover_primitives::{Query, VerifierExitStatus, STARK_PROGRAM_V3_HASH};
use prover_primitives::stark_program_auth::StarkProgramMetadataStorage;

use frame_support::pallet_prelude::DispatchError::Other;
use frame_support::{assert_err, assert_noop, assert_ok};
use sp_runtime::traits::BadOrigin;

use crate::mock::{ExtBuilder, ProverModule, RuntimeOrigin, System, Test};

fn prover_configured_in_genesis() -> RuntimeOrigin {
    RuntimeOrigin::signed(PROVER_3)
}

#[test]
fn submit_proof_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let proof = b"".to_vec();
        let query = Query {
            chain_id: 1,
            height: 1,
            index: 1,
            layout_segments: vec![],
        };

        assert_noop!(
            ProverModule::submit_proof(RuntimeOrigin::none(), proof, query),
            BadOrigin
        );
    });
}

#[test]
fn submit_proof_should_error_when_proof_is_empty() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let query = Query {
            chain_id: 1,
            height: 1,
            index: 1,
            layout_segments: vec![],
        };

        assert_noop!(
            ProverModule::submit_proof(prover_configured_in_genesis(), b"".to_vec(), query),
            Error::<Test>::InvalidProofSubmitted
        );
    })
}

// this test additionally logs an error since it's unable to verify the proof
#[test]
fn submit_proof_should_error_when_proof_is_not_empty_but_not_valid() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_ok!(ProverModule::set_stark_program_metadata(
            RuntimeOrigin::root(),
            STARK_PROGRAM_V3_HASH,
            1
        ));

        let proof = b"abcd".to_vec();
        let query = Query {
            chain_id: 1,
            height: 1,
            index: 1,
            layout_segments: vec![],
        };

        assert_noop!(
            ProverModule::submit_proof(RuntimeOrigin::signed(PROVER_3), proof, query),
            Error::<Test>::InvalidProofSubmitted
        );
    })
}

#[test]
fn submit_proof_should_ok_and_emit_an_event_when_input_is_valid_and_stark_metadata_set_correctly() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_ok!(ProverModule::set_stark_program_metadata(
            RuntimeOrigin::root(),
            STARK_PROGRAM_V3_HASH,
            1
        ));

        let proof = std::fs::read("../../cairo/stone-verifier/proof_example.json")
            .expect("Proof example to be there");

        let query = Query {
            chain_id: 1,
            height: 1,
            index: 1,
            layout_segments: vec![],
        };

        assert_ok!(ProverModule::submit_proof(
            RuntimeOrigin::signed(PROVER_3),
            proof,
            query.clone()
        ),);

        // assert on storage change
        assert_eq!(
            QueryResultById::<Test>::get(query.id()),
            Some(VerifierExitStatus::Success)
        );

        // assert on emited event
        System::assert_last_event(
            Event::QueryVerified(query.id(), PROVER_3, VerifierExitStatus::Success).into(),
        );
    })
}

#[test]
fn submit_proof_should_error_when_stark_metadata_not_set() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        // using some random incorrect proof because the verification will error out at
        // metadata not set before reaching the proof part
        let proof = vec![0; 10];

        let query = Query {
            chain_id: 1,
            height: 1,
            index: 1,
            layout_segments: vec![],
        };

        assert_err!(
            ProverModule::submit_proof(RuntimeOrigin::signed(PROVER_3), proof, query.clone()),
            Error::<Test>::StarkMetadataNotSet
        );

        // assert on storage change
        assert_eq!(QueryResultById::<Test>::get(query.id()), None);
    })
}

#[test]
fn submit_proof_should_error_when_stark_metadata_version_is_incorrect() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_ok!(ProverModule::set_stark_program_metadata(
            RuntimeOrigin::root(),
            1,
            1
        ));

        let proof = std::fs::read("../../cairo/stone-verifier/proof_example.json")
            .expect("Proof example to be there");

        let query = Query {
            chain_id: 1,
            height: 1,
            index: 1,
            layout_segments: vec![],
        };

        assert_noop!(
            ProverModule::submit_proof(RuntimeOrigin::signed(PROVER_3), proof, query),
            Error::<Test>::InvalidProofSubmitted
        );
    })
}

#[test]
fn set_stark_program_metadata_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_noop!(
            ProverModule::set_stark_program_metadata(
                RuntimeOrigin::none(),
                StarkProgramMetadataStorage::AUTH_HASH_V3_DEV,
                StarkProgramMetadataStorage::V3_DEV
            ),
            BadOrigin
        );
    });
}

#[test]
fn set_stark_program_metadata_should_error_when_not_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_noop!(
            ProverModule::set_stark_program_metadata(
                RuntimeOrigin::signed(4),
                StarkProgramMetadataStorage::AUTH_HASH_V3_DEV,
                StarkProgramMetadataStorage::V3_DEV
            ),
            BadOrigin
        );
    });
}

#[test]
fn set_stark_program_metadata_should_error_when_program_version_already_set() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_ok!(ProverModule::set_stark_program_metadata(
            RuntimeOrigin::root(),
            StarkProgramMetadataStorage::AUTH_HASH_V3_DEV,
            StarkProgramMetadataStorage::V3_DEV
        ));

        // already set above, can't set it twice
        const EXPECTED_ERROR_MESSAGE: &str = "Program version already exists";
        assert_noop!(
            ProverModule::set_stark_program_metadata(
                RuntimeOrigin::root(),
                StarkProgramMetadataStorage::AUTH_HASH_V3_DEV,
                StarkProgramMetadataStorage::V3_DEV
            ),
            Other(EXPECTED_ERROR_MESSAGE)
        );
    });
}

#[test]
fn set_stark_program_metadata_should_update_storage_and_emit_an_event() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        // initial state
        assert_eq!(ProverModule::last_version(), 0);

        assert_ok!(ProverModule::set_stark_program_metadata(
            RuntimeOrigin::root(),
            StarkProgramMetadataStorage::AUTH_HASH_V3_DEV,
            StarkProgramMetadataStorage::V3_DEV
        ));

        // assert on storage
        assert_eq!(
            ProverModule::last_version(),
            StarkProgramMetadataStorage::V3_DEV
        );

        // assert on emited event
        System::assert_last_event(
            crate::Event::StarkProgramMetadataSet(
                StarkProgramMetadataStorage::V3_DEV,
                StarkProgramMetadataStorage::AUTH_HASH_V3_DEV,
            )
            .into(),
        );
    });
}

#[test]
fn set_stark_program_metadata_can_be_called_twice_with_different_value() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        // initial state
        assert_ok!(ProverModule::set_stark_program_metadata(
            RuntimeOrigin::root(),
            StarkProgramMetadataStorage::AUTH_HASH_V2_DEV,
            StarkProgramMetadataStorage::V2_DEV
        ));

        assert_eq!(
            ProverModule::last_version(),
            StarkProgramMetadataStorage::V2_DEV
        );

        System::assert_last_event(
            crate::Event::StarkProgramMetadataSet(
                StarkProgramMetadataStorage::V2_DEV,
                StarkProgramMetadataStorage::AUTH_HASH_V2_DEV,
            )
            .into(),
        );

        // call it again to upgrade the value
        // NOTE: currently a downgrade is also supported
        assert_ok!(ProverModule::set_stark_program_metadata(
            RuntimeOrigin::root(),
            StarkProgramMetadataStorage::AUTH_HASH_V3_DEV,
            StarkProgramMetadataStorage::V3_DEV
        ));

        // assert on storage
        assert_eq!(
            ProverModule::last_version(),
            StarkProgramMetadataStorage::V3_DEV
        );

        // assert on emited event
        System::assert_last_event(
            crate::Event::StarkProgramMetadataSet(
                StarkProgramMetadataStorage::V3_DEV,
                StarkProgramMetadataStorage::AUTH_HASH_V3_DEV,
            )
            .into(),
        );
    });
}
