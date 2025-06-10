use super::*;
use pallet_prover_primitives::{
    STARK_PROGRAM_V1_HASH, STARK_PROGRAM_V2_HASH, STARK_PROGRAM_V3_HASH,
};

use frame_support::{assert_noop, assert_ok};
use sp_core::H256;
use sp_runtime::traits::BadOrigin;

use crate::mock::{ExtBuilder, ProverModule, RuntimeOrigin, System, Test};

#[test]
fn set_stark_program_metadata_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_noop!(
            ProverModule::set_stark_program_metadata(
                RuntimeOrigin::none(),
                2,
                STARK_PROGRAM_V3_HASH,
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
                3,
                STARK_PROGRAM_V3_HASH
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
            3,
            STARK_PROGRAM_V3_HASH
        ));

        // already set above, can't set it twice
        assert_noop!(
            ProverModule::set_stark_program_metadata(
                RuntimeOrigin::root(),
                3,
                STARK_PROGRAM_V3_HASH
            ),
            Error::<Test>::StarkProgramMetadataAlreadySet
        );
    });
}

#[test]
fn set_stark_program_metadata_should_update_storage_and_emit_an_event() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_ok!(ProverModule::set_stark_program_metadata(
            RuntimeOrigin::root(),
            2,
            STARK_PROGRAM_V2_HASH,
        ));

        let meta = StarkProgramMetadata::<Test>::get(2);
        assert_eq!(meta, STARK_PROGRAM_V2_HASH);

        // assert on emited event
        System::assert_last_event(
            crate::Event::StarkProgramMetadataSet(2, STARK_PROGRAM_V2_HASH).into(),
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
            1,
            STARK_PROGRAM_V1_HASH,
        ));

        System::assert_last_event(
            crate::Event::StarkProgramMetadataSet(1, STARK_PROGRAM_V1_HASH).into(),
        );

        // call it again to upgrade the value
        // NOTE: currently a downgrade is also supported
        assert_ok!(ProverModule::set_stark_program_metadata(
            RuntimeOrigin::root(),
            2,
            STARK_PROGRAM_V2_HASH,
        ));

        // assert on emited event
        System::assert_last_event(
            crate::Event::StarkProgramMetadataSet(2, STARK_PROGRAM_V2_HASH).into(),
        );
    });
}

#[test]
fn set_start_program_metadata_and_remove_works() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_ok!(ProverModule::set_stark_program_metadata(
            RuntimeOrigin::root(),
            1,
            STARK_PROGRAM_V1_HASH,
        ));

        System::assert_last_event(
            crate::Event::StarkProgramMetadataSet(1, STARK_PROGRAM_V1_HASH).into(),
        );

        assert_ok!(ProverModule::remove_stark_program_metadata(
            RuntimeOrigin::root(),
            1,
        ));

        assert_eq!(StarkProgramMetadata::<Test>::get(1), H256::zero());

        System::assert_last_event(crate::Event::StarkProgramMetadataRemoved(1).into());
    });
}

#[test]
fn cant_remove_unset_stark_program_metadata() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_noop!(
            ProverModule::remove_stark_program_metadata(RuntimeOrigin::root(), 1),
            Error::<Test>::StarkProgramMetadataNotFound
        );
    });
}
