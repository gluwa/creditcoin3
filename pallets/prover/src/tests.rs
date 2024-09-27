use self::mock::PROVER_3;

use super::*;
use pallet_prover_primitives::{Query, VerifierExitStatus, STARK_PROGRAM_V3_HASH};

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
fn submit_proof_should_error_when_proof_is_not_empty_but_not_valid_and_stark_metadata_is_set() {
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
fn submit_proof_should_err_and_when_input_is_valid_but_stark_metadata_not_set() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let proof = std::fs::read("../../cairo/stone-verifier/proof_example.json")
            .expect("Proof example to be there");

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
fn submit_proof_should_err_and_when_input_is_valid_but_stark_metadata_set_incorrectly() {
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
