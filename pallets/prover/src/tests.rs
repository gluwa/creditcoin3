use self::mock::PROVER_3;

use super::*;
use prover_primitives::{Query, VerifierExitStatus};

use frame_support::{assert_noop, assert_ok};
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

#[test]
fn submit_proof_should_error_when_proof_is_not_empty_but_not_valid() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        // mock::verify_proof() designates this as invalid b/c it is < 10 chars
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
fn submit_proof_should_ok_and_emit_an_event_when_input_is_valid() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        // mock::verify_proof() designates this as valid b/c it is >= 10 chars
        let proof = b"0123456789".to_vec();
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
            crate::Event::QueryVerified(query.id(), PROVER_3, VerifierExitStatus::Success).into(),
        );
    })
}
