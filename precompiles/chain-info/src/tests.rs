use crate::{
    mock::{
        Account::{Alice, Precompile},
        *,
    },
    BoundsCheckResult, ChainInfo, ChainInfoResult, HashResult, HeightHashResult, HeightResult,
};

use attestor_primitives::{AttestationCheckpoint, AttestationData, SignedAttestation};

use pallet_attestation_poc::{Attestations, Checkpoints, LastCheckpoint, LastDigest};
use precompile_utils::{prelude::UnboundedBytes, testing::*};

use sp_core::{H160, H256};

fn precompiles() -> Precompiles<Runtime> {
    PrecompilesValue::get()
}

fn create_dummy_attestation(height: u64) -> SignedAttestation<H256, AccountId> {
    let attestation = AttestationData {
        chain_key: SUPPORTED_CHAIN_KEY,
        header_number: height,
        header_hash: H256::random(),
        root: H256::zero(),
        prev_digest: None,
    };

    SignedAttestation {
        attestation,
        signature: [0; 96],
        attestors: vec![],
        continuity_proof: Default::default(),
    }
}

// exercises the scenario where input data is invalid
#[test]
fn get_supported_chains_works() {
    let alice: H160 = Alice.into();

    let expected_result = vec![ChainInfo {
        chain_key: SUPPORTED_CHAIN_KEY,
        chain_id: SUPPORTED_CHAIN_ID,
        chain_name: UnboundedBytes::from(SUPPORTED_CHAIN_NAME),
        chain_encoding: SUPPORTED_CHAIN_ENCODING as u8,
    }];

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(alice, Precompile, PCall::get_supported_chains {})
                .execute_returns(expected_result);
        });
}

#[test]
fn get_chain_by_key_works() {
    let alice: H160 = Alice.into();

    let expected_result = ChainInfoResult {
        chain: ChainInfo {
            chain_key: SUPPORTED_CHAIN_KEY,
            chain_id: SUPPORTED_CHAIN_ID,
            chain_name: UnboundedBytes::from(SUPPORTED_CHAIN_NAME),
            chain_encoding: SUPPORTED_CHAIN_ENCODING as u8,
        },
        exists: true,
    };

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::get_chain_by_key {
                        chain_key: SUPPORTED_CHAIN_KEY,
                    },
                )
                .execute_returns(expected_result);
        });
}

#[test]
fn get_attestation_genesis_height_works() {
    let alice: H160 = Alice.into();

    let expected_result: u64 = 23;

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            pallet_attestation_poc::AttestationChainGenesisBlockNumber::<Runtime>::insert(
                SUPPORTED_CHAIN_KEY,
                expected_result,
            );

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::get_attestation_genesis_height {
                        chain_key: SUPPORTED_CHAIN_KEY,
                    },
                )
                .execute_returns(expected_result);
        });
}

#[test]
fn get_chain_by_key_returns_default_data_with_unknown_chain_key() {
    let alice: H160 = Alice.into();

    let unknown_supported_chain_key: u64 = 9999;

    let expected_result = ChainInfoResult {
        chain: ChainInfo::default(),
        exists: false,
    };

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::get_chain_by_key {
                        chain_key: unknown_supported_chain_key,
                    },
                )
                .execute_returns(expected_result);
        });
}

#[test]
fn get_latest_attestation_height_and_hash_works() {
    let alice: H160 = Alice.into();

    let fake_height: u64 = 1000;
    let fake_digest = H256::from_slice(&[23_u8; 32]);

    let expected_result = HeightHashResult {
        height: fake_height,
        hash: fake_digest,
        is_attestation: true,
        exists: true,
    };

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            // Inserting fake data into storage for testing
            LastDigest::<Runtime>::insert(SUPPORTED_CHAIN_KEY, (fake_height, fake_digest));

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::get_latest_attestation_height_and_hash {
                        chain_key: SUPPORTED_CHAIN_KEY,
                    },
                )
                .execute_returns(expected_result);
        });
}

#[test]
fn get_latest_attestation_height_and_hash_returns_default_when_no_data() {
    let alice: H160 = Alice.into();

    let expected_result = HeightHashResult::default();

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            // No attestation data in storage - should return default
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::get_latest_attestation_height_and_hash {
                        chain_key: SUPPORTED_CHAIN_KEY,
                    },
                )
                .execute_returns(expected_result);
        });
}

#[test]
fn get_latest_checkpoint_height_and_hash_works() {
    let alice: H160 = Alice.into();

    let fake_height: u64 = 2000;
    let fake_digest = H256::from_slice(&[45_u8; 32]);

    let fake_attestation_checkpoint = AttestationCheckpoint {
        block_number: fake_height,
        digest: fake_digest,
    };

    let expected_result = HeightHashResult {
        height: fake_height,
        hash: fake_digest,
        is_attestation: false,
        exists: true,
    };

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            // Inserting fake data into storage for testing
            LastCheckpoint::<Runtime>::insert(SUPPORTED_CHAIN_KEY, fake_attestation_checkpoint);

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::get_latest_checkpoint_height_and_hash {
                        chain_key: SUPPORTED_CHAIN_KEY,
                    },
                )
                .execute_returns(expected_result);
        });
}

#[test]
fn get_latest_checkpoint_height_and_hash_returns_default_when_no_data() {
    let alice: H160 = Alice.into();

    let expected_result = HeightHashResult::default();

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            // No checkpoint data in storage - should return default
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::get_latest_checkpoint_height_and_hash {
                        chain_key: SUPPORTED_CHAIN_KEY,
                    },
                )
                .execute_returns(expected_result);
        });
}

#[test]
fn find_highest_attested_before_works() {
    let alice: H160 = Alice.into();

    let dummy_height: u64 = 900;
    let dummy_attestation = create_dummy_attestation(dummy_height);

    let expected_result = HeightHashResult {
        height: dummy_height,
        hash: dummy_attestation.digest(),
        is_attestation: true,
        exists: true,
    };

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            Attestations::<Runtime>::insert(
                SUPPORTED_CHAIN_KEY,
                dummy_attestation.digest(),
                dummy_attestation,
            );

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::find_highest_attested_before {
                        chain_key: SUPPORTED_CHAIN_KEY,
                        target_height: dummy_height + 1,
                    },
                )
                .execute_returns(expected_result);
        });
}

#[test]
fn find_lowest_attested_after_works() {
    let alice: H160 = Alice.into();

    let dummy_height: u64 = 900;
    let dummy_attestation = create_dummy_attestation(dummy_height);

    let expected_result = HeightHashResult {
        height: dummy_height,
        hash: dummy_attestation.digest(),
        is_attestation: true,
        exists: true,
    };

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            Attestations::<Runtime>::insert(
                SUPPORTED_CHAIN_KEY,
                dummy_attestation.digest(),
                dummy_attestation,
            );
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::find_lowest_attested_after {
                        chain_key: SUPPORTED_CHAIN_KEY,
                        target_height: dummy_height - 1,
                    },
                )
                .execute_returns(expected_result);
        });
}

#[test]
fn is_height_attested_works() {
    let alice: H160 = Alice.into();

    let dummy_height_from: u64 = 900;
    let dummy_attestation_from = create_dummy_attestation(dummy_height_from);

    let dummy_height_to: u64 = 924;
    let dummy_attestation_to = create_dummy_attestation(dummy_height_to);

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            Attestations::<Runtime>::insert(
                SUPPORTED_CHAIN_KEY,
                dummy_attestation_from.digest(),
                dummy_attestation_from,
            );

            Attestations::<Runtime>::insert(
                SUPPORTED_CHAIN_KEY,
                dummy_attestation_to.digest(),
                dummy_attestation_to,
            );

            // In this check since dummy_height_from + 1 is between dummy_height_from and dummy_height_to, it should be attested
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::is_height_attested {
                        chain_key: SUPPORTED_CHAIN_KEY,
                        target_height: dummy_height_from + 1,
                    },
                )
                .execute_returns(true);

            // In this checck since dummy_height_to + 1 is above dummy_height_to, it should not be attested
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::is_height_attested {
                        chain_key: SUPPORTED_CHAIN_KEY,
                        target_height: dummy_height_to + 1,
                    },
                )
                .execute_returns(false);
        });
}

#[test]
fn get_attestation_bounds_works() {
    let alice: H160 = Alice.into();

    let dummy_height_from: u64 = 900;
    let dummy_attestation_from = create_dummy_attestation(dummy_height_from);
    let dummy_digest_from = dummy_attestation_from.digest();

    let dummy_height_to: u64 = 924;
    let dummy_attestation_to = create_dummy_attestation(dummy_height_to);
    let dummy_digest_to = dummy_attestation_to.digest();

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            Attestations::<Runtime>::insert(
                SUPPORTED_CHAIN_KEY,
                dummy_attestation_from.digest(),
                dummy_attestation_from,
            );

            Attestations::<Runtime>::insert(
                SUPPORTED_CHAIN_KEY,
                dummy_attestation_to.digest(),
                dummy_attestation_to,
            );

            let expected_result = BoundsCheckResult {
                parent: dummy_height_from,
                parent_hash: dummy_digest_from,
                parent_is_attestation: true,
                child: dummy_height_to,
                child_hash: dummy_digest_to,
                child_is_attestation: true,
                is_attested: true,
            };

            // In this check since we have both a parent and a child attestation around the target height, we get all data filled
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::get_attestation_bounds {
                        chain_key: SUPPORTED_CHAIN_KEY,
                        target_height: dummy_height_from + 10,
                    },
                )
                .execute_returns(expected_result);

            let expected_result = BoundsCheckResult {
                parent: dummy_height_to,
                parent_hash: dummy_digest_to,
                parent_is_attestation: true,
                child: u64::default(),
                child_hash: H256::default(),
                child_is_attestation: false,
                is_attested: false,
            };

            // In this check since there is no child attestation after the target height, child data should be defaulted
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::get_attestation_bounds {
                        chain_key: SUPPORTED_CHAIN_KEY,
                        target_height: dummy_height_to + 10,
                    },
                )
                .execute_returns(expected_result);

            let expected_result = BoundsCheckResult {
                parent: u64::default(),
                parent_hash: H256::default(),
                parent_is_attestation: false,
                child: dummy_height_from,
                child_hash: dummy_digest_from,
                child_is_attestation: true,
                is_attested: false,
            };

            // In this check since there is no parent attestation before the target height, parent data should be defaulted
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::get_attestation_bounds {
                        chain_key: SUPPORTED_CHAIN_KEY,
                        target_height: dummy_height_from - 10,
                    },
                )
                .execute_returns(expected_result);
        });
}

#[test]
fn get_attestation_height_by_digest_works() {
    let alice: H160 = Alice.into();

    let expected_result = HeightResult {
        height: 1234,
        exists: true,
    };

    let digest = H256::from_slice(&[56_u8; 32]);

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            Attestations::<Runtime>::insert(
                SUPPORTED_CHAIN_KEY,
                digest,
                create_dummy_attestation(1234),
            );

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::get_attestation_height_for_digest {
                        chain_key: SUPPORTED_CHAIN_KEY,
                        digest,
                    },
                )
                .execute_returns(expected_result);
        });
}

#[test]
fn get_attestation_height_by_digest_returns_empty_when_no_attestation_at_query_height() {
    let alice: H160 = Alice.into();

    let expected_result = HeightResult::default();

    let digest = H256::from_slice(&[99_u8; 32]);

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            // No attestation data in storage - should return default
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::get_attestation_height_for_digest {
                        chain_key: SUPPORTED_CHAIN_KEY,
                        digest,
                    },
                )
                .execute_returns(expected_result);
        });
}

#[test]
fn get_checkpoint_by_height_works() {
    let alice: H160 = Alice.into();

    let fake_height: u64 = 2000;
    let fake_digest = H256::from_slice(&[45_u8; 32]);

    let expected_result = HashResult {
        hash: fake_digest,
        exists: true,
    };

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            // Inserting fake data into storage for testing
            Checkpoints::<Runtime>::insert(SUPPORTED_CHAIN_KEY, fake_height, fake_digest);

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::get_checkpoint_for_height {
                        chain_key: SUPPORTED_CHAIN_KEY,
                        height: fake_height,
                    },
                )
                .execute_returns(expected_result);
        });
}

#[test]
fn get_checkpoint_by_height_returns_default_when_no_checkpoint_at_query_height() {
    let alice: H160 = Alice.into();
    let fake_height: u64 = 3000;
    let expected_result = HashResult::default();

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            // No checkpoint data in storage - should return default
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::get_checkpoint_for_height {
                        chain_key: SUPPORTED_CHAIN_KEY,
                        height: fake_height,
                    },
                )
                .execute_returns(expected_result);
        });
}
