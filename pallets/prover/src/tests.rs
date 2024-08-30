use self::mock::PROVER_3;

use super::*;
use prover_primitives::Query;

use frame_support::assert_err;

use crate::mock::{ExtBuilder, ProverModule, RuntimeOrigin, Test};

fn prover_configured_in_genesis() -> RuntimeOrigin {
    RuntimeOrigin::signed(PROVER_3)
}

#[test]
fn submit_proof_should_error_with_invalid_input() {
    ExtBuilder.build_and_execute(|| {
        let query = Query {
            chain_id: 1,
            height: 1,
            index: 1,
            layout_segments: vec![],
        };

        assert_err!(
            ProverModule::submit_proof(prover_configured_in_genesis(), b"".to_vec(), query),
            Error::<Test>::InvalidProofSubmitted
        );
    })
}
