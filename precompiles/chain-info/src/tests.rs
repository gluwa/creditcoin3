use crate::{
    mock::{
        Account::{Alice, Precompile},
        *,
    },
    BoundsCheckResult, ChainInfo, ChainInfoResult, HashResult, HeightHashResult, HeightResult,
};

use attestor_primitives::{AttestationCheckpoint, AttestationData, SignedAttestation};

use pallet_attestation::{
    Attestations, CheckpointBuckets, Checkpoints, LastCheckpoint, LastDigest,
    Pallet as AttestationPallet,
};
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
            pallet_attestation::AttestationChainGenesisBlockNumber::<Runtime>::insert(
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

// Regression: `CheckpointBuckets` and `Checkpoints` are written/removed as a pair, but the
// prefix-clear paths in `clear_or_revert.rs` advance their cursors independently after a chain
// removal, so a bucket entry can briefly outlive its checkpoint. The bucket-search precompiles
// must skip such an orphan rather than `.unwrap()` on the missing checkpoint — a panic here
// would trap runtime WASM and halt block import (DoS).
#[test]
fn find_highest_attested_before_skips_orphan_bucket_entry() {
    let alice: H160 = Alice.into();

    let target_height: u64 = 1500;
    let orphan_height: u64 = 1400; // < target, sits in the pivot bucket just below target
    let valid_height: u64 = 800; // < target, in the next pivot down, has a real checkpoint
    let valid_digest = H256::from_slice(&[7_u8; 32]);
    let last_checkpoint_height: u64 = 2000; // >= target, forces the bucket-search branch

    let expected_result = HeightHashResult {
        height: valid_height,
        hash: valid_digest,
        is_attestation: false,
        exists: true,
    };

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            LastCheckpoint::<Runtime>::insert(
                SUPPORTED_CHAIN_KEY,
                AttestationCheckpoint {
                    block_number: last_checkpoint_height,
                    digest: H256::random(),
                },
            );

            // Orphan bucket entry with no matching `Checkpoints` value: the desync window.
            let orphan_pivot = AttestationPallet::<Runtime>::compute_block_index_for(orphan_height);
            CheckpointBuckets::<Runtime>::insert(
                (SUPPORTED_CHAIN_KEY, orphan_pivot, orphan_height),
                (),
            );
            assert!(Checkpoints::<Runtime>::get(SUPPORTED_CHAIN_KEY, orphan_height).is_none());

            // A consistent checkpoint one pivot lower; the search should skip the orphan and
            // fall through to this one.
            let valid_pivot = AttestationPallet::<Runtime>::compute_block_index_for(valid_height);
            CheckpointBuckets::<Runtime>::insert(
                (SUPPORTED_CHAIN_KEY, valid_pivot, valid_height),
                (),
            );
            Checkpoints::<Runtime>::insert(SUPPORTED_CHAIN_KEY, valid_height, valid_digest);

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::find_highest_attested_before {
                        chain_key: SUPPORTED_CHAIN_KEY,
                        target_height,
                    },
                )
                .execute_returns(expected_result);
        });
}

#[test]
fn find_lowest_attested_after_skips_orphan_bucket_entry() {
    let alice: H160 = Alice.into();

    let target_height: u64 = 1500;
    let orphan_height: u64 = 1600; // >= target, sits in the pivot bucket at target
    let valid_height: u64 = 2400; // >= target, in the next pivot up, has a real checkpoint
    let valid_digest = H256::from_slice(&[9_u8; 32]);
    let last_checkpoint_height: u64 = 3000; // > target, forces the bucket-search branch

    let expected_result = HeightHashResult {
        height: valid_height,
        hash: valid_digest,
        is_attestation: false,
        exists: true,
    };

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            LastCheckpoint::<Runtime>::insert(
                SUPPORTED_CHAIN_KEY,
                AttestationCheckpoint {
                    block_number: last_checkpoint_height,
                    digest: H256::random(),
                },
            );

            // Orphan bucket entry with no matching `Checkpoints` value: the desync window.
            let orphan_pivot = AttestationPallet::<Runtime>::compute_block_index_for(orphan_height);
            CheckpointBuckets::<Runtime>::insert(
                (SUPPORTED_CHAIN_KEY, orphan_pivot, orphan_height),
                (),
            );
            assert!(Checkpoints::<Runtime>::get(SUPPORTED_CHAIN_KEY, orphan_height).is_none());

            // A consistent checkpoint one pivot higher; the search should skip the orphan and
            // fall through to this one.
            let valid_pivot = AttestationPallet::<Runtime>::compute_block_index_for(valid_height);
            CheckpointBuckets::<Runtime>::insert(
                (SUPPORTED_CHAIN_KEY, valid_pivot, valid_height),
                (),
            );
            Checkpoints::<Runtime>::insert(SUPPORTED_CHAIN_KEY, valid_height, valid_digest);

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::find_lowest_attested_after {
                        chain_key: SUPPORTED_CHAIN_KEY,
                        target_height,
                    },
                )
                .execute_returns(expected_result);
        });
}

// Regression for the within-bucket case: a single pivot can hold several heights and only
// the extremal one may be orphaned. The search must probe the remaining heights in the same
// bucket before moving on, otherwise callers get a wrong (further-pivot) height or an empty
// result even though a valid checkpoint exists in the bucket.
#[test]
fn find_highest_attested_before_probes_within_bucket_past_orphan() {
    let alice: H160 = Alice.into();

    let target_height: u64 = 1500;
    let orphan_height: u64 = 1400; // extremal-below in the target bucket, but orphaned
    let valid_in_bucket: u64 = 1200; // same bucket (pivot 1000), has a real checkpoint
    let valid_digest = H256::from_slice(&[11_u8; 32]);
    // A valid checkpoint one pivot lower acts as a distractor: if the search wrongly
    // abandoned the bucket it would return this instead.
    let distractor_height: u64 = 800;
    let distractor_digest = H256::from_slice(&[22_u8; 32]);
    let last_checkpoint_height: u64 = 2000; // >= target, forces the bucket-search branch

    let expected_result = HeightHashResult {
        height: valid_in_bucket,
        hash: valid_digest,
        is_attestation: false,
        exists: true,
    };

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            LastCheckpoint::<Runtime>::insert(
                SUPPORTED_CHAIN_KEY,
                AttestationCheckpoint {
                    block_number: last_checkpoint_height,
                    digest: H256::random(),
                },
            );

            let pivot = AttestationPallet::<Runtime>::compute_block_index_for(orphan_height);
            // Orphan: bucket entry, no checkpoint.
            CheckpointBuckets::<Runtime>::insert((SUPPORTED_CHAIN_KEY, pivot, orphan_height), ());
            // Valid, same bucket.
            CheckpointBuckets::<Runtime>::insert((SUPPORTED_CHAIN_KEY, pivot, valid_in_bucket), ());
            Checkpoints::<Runtime>::insert(SUPPORTED_CHAIN_KEY, valid_in_bucket, valid_digest);

            // Distractor in the next pivot down.
            let distractor_pivot =
                AttestationPallet::<Runtime>::compute_block_index_for(distractor_height);
            CheckpointBuckets::<Runtime>::insert(
                (SUPPORTED_CHAIN_KEY, distractor_pivot, distractor_height),
                (),
            );
            Checkpoints::<Runtime>::insert(
                SUPPORTED_CHAIN_KEY,
                distractor_height,
                distractor_digest,
            );

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::find_highest_attested_before {
                        chain_key: SUPPORTED_CHAIN_KEY,
                        target_height,
                    },
                )
                .execute_returns(expected_result);
        });
}

#[test]
fn find_lowest_attested_after_probes_within_bucket_past_orphan() {
    let alice: H160 = Alice.into();

    let target_height: u64 = 1500;
    let orphan_height: u64 = 1600; // extremal-above in the target bucket, but orphaned
    let valid_in_bucket: u64 = 1800; // same bucket (pivot 1000), has a real checkpoint
    let valid_digest = H256::from_slice(&[33_u8; 32]);
    // Distractor one pivot higher: returned only if the bucket were wrongly abandoned.
    let distractor_height: u64 = 2400;
    let distractor_digest = H256::from_slice(&[44_u8; 32]);
    let last_checkpoint_height: u64 = 3000; // > target, forces the bucket-search branch

    let expected_result = HeightHashResult {
        height: valid_in_bucket,
        hash: valid_digest,
        is_attestation: false,
        exists: true,
    };

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            LastCheckpoint::<Runtime>::insert(
                SUPPORTED_CHAIN_KEY,
                AttestationCheckpoint {
                    block_number: last_checkpoint_height,
                    digest: H256::random(),
                },
            );

            let pivot = AttestationPallet::<Runtime>::compute_block_index_for(target_height);
            // Orphan: bucket entry, no checkpoint.
            CheckpointBuckets::<Runtime>::insert((SUPPORTED_CHAIN_KEY, pivot, orphan_height), ());
            // Valid, same bucket.
            CheckpointBuckets::<Runtime>::insert((SUPPORTED_CHAIN_KEY, pivot, valid_in_bucket), ());
            Checkpoints::<Runtime>::insert(SUPPORTED_CHAIN_KEY, valid_in_bucket, valid_digest);

            // Distractor in the next pivot up.
            let distractor_pivot =
                AttestationPallet::<Runtime>::compute_block_index_for(distractor_height);
            CheckpointBuckets::<Runtime>::insert(
                (SUPPORTED_CHAIN_KEY, distractor_pivot, distractor_height),
                (),
            );
            Checkpoints::<Runtime>::insert(
                SUPPORTED_CHAIN_KEY,
                distractor_height,
                distractor_digest,
            );

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::find_lowest_attested_after {
                        chain_key: SUPPORTED_CHAIN_KEY,
                        target_height,
                    },
                )
                .execute_returns(expected_result);
        });
}

// Regression for the sparse-checkpoint false negative (#18): the bucket walk used to give up
// after a fixed 5 buckets, so a valid checkpoint more than 5 buckets away from the target was
// never found and the call wrongly reported "not attested". The walk is now bounded by the real
// distance to the last checkpoint, so a distant-but-valid checkpoint is found.
#[test]
fn find_highest_attested_before_finds_checkpoint_more_than_five_buckets_away() {
    let alice: H160 = Alice.into();

    let target_height: u64 = 10_000; // pivot 10_000
                                     // Valid checkpoint ~8 buckets below the target pivot \u2014 beyond the old fixed 5-bucket limit.
    let valid_height: u64 = 2_000; // pivot 2_000 => 8 buckets down from 10_000
    let valid_digest = H256::from_slice(&[55_u8; 32]);
    let last_checkpoint_height: u64 = 20_000; // >= target, forces the bucket-search branch

    let expected_result = HeightHashResult {
        height: valid_height,
        hash: valid_digest,
        is_attestation: false,
        exists: true,
    };

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            LastCheckpoint::<Runtime>::insert(
                SUPPORTED_CHAIN_KEY,
                AttestationCheckpoint {
                    block_number: last_checkpoint_height,
                    digest: H256::random(),
                },
            );

            let valid_pivot = AttestationPallet::<Runtime>::compute_block_index_for(valid_height);
            CheckpointBuckets::<Runtime>::insert(
                (SUPPORTED_CHAIN_KEY, valid_pivot, valid_height),
                (),
            );
            Checkpoints::<Runtime>::insert(SUPPORTED_CHAIN_KEY, valid_height, valid_digest);

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::find_highest_attested_before {
                        chain_key: SUPPORTED_CHAIN_KEY,
                        target_height,
                    },
                )
                .execute_returns(expected_result);
        });
}

// Symmetric regression for #18 on the upward walk.
#[test]
fn find_lowest_attested_after_finds_checkpoint_more_than_five_buckets_away() {
    let alice: H160 = Alice.into();

    let target_height: u64 = 2_000; // pivot 2_000
                                    // Valid checkpoint ~8 buckets above the target pivot \u2014 beyond the old fixed 5-bucket limit.
    let valid_height: u64 = 10_000; // pivot 10_000 => 8 buckets up from 2_000
    let valid_digest = H256::from_slice(&[66_u8; 32]);
    let last_checkpoint_height: u64 = 20_000; // > target, forces the bucket-search branch

    let expected_result = HeightHashResult {
        height: valid_height,
        hash: valid_digest,
        is_attestation: false,
        exists: true,
    };

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            LastCheckpoint::<Runtime>::insert(
                SUPPORTED_CHAIN_KEY,
                AttestationCheckpoint {
                    block_number: last_checkpoint_height,
                    digest: H256::random(),
                },
            );

            let valid_pivot = AttestationPallet::<Runtime>::compute_block_index_for(valid_height);
            CheckpointBuckets::<Runtime>::insert(
                (SUPPORTED_CHAIN_KEY, valid_pivot, valid_height),
                (),
            );
            Checkpoints::<Runtime>::insert(SUPPORTED_CHAIN_KEY, valid_height, valid_digest);

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::find_lowest_attested_after {
                        chain_key: SUPPORTED_CHAIN_KEY,
                        target_height,
                    },
                )
                .execute_returns(expected_result);
        });
}

// Bugbot regression (PR #1108): the downward walk in `find_highest_attested_before` must be
// bounded by `target_height` (distance to genesis), NOT `abs_diff(target, last_checkpoint)`.
// Here the last checkpoint sits just ONE block above the target, so the old `abs_diff` bound was
// only ~1 bucket — yet the nearest valid checkpoint below the target is ~8 buckets down. With the
// wrong bound the search stopped early and falsely reported "not attested".
#[test]
fn find_highest_attested_before_finds_far_checkpoint_when_last_checkpoint_just_above_target() {
    let alice: H160 = Alice.into();

    let target_height: u64 = 10_000; // pivot 10_000
    let valid_height: u64 = 2_000; // pivot 2_000 => 8 buckets below the target pivot
    let valid_digest = H256::from_slice(&[77_u8; 32]);
    // Last checkpoint is only ONE block above the target: abs_diff(10_000, 10_001) == 1, which
    // under the old buggy bound capped the walk at ~1 bucket and missed `valid_height`.
    let last_checkpoint_height: u64 = 10_001;

    let expected_result = HeightHashResult {
        height: valid_height,
        hash: valid_digest,
        is_attestation: false,
        exists: true,
    };

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            LastCheckpoint::<Runtime>::insert(
                SUPPORTED_CHAIN_KEY,
                AttestationCheckpoint {
                    block_number: last_checkpoint_height,
                    digest: H256::random(),
                },
            );

            let valid_pivot = AttestationPallet::<Runtime>::compute_block_index_for(valid_height);
            CheckpointBuckets::<Runtime>::insert(
                (SUPPORTED_CHAIN_KEY, valid_pivot, valid_height),
                (),
            );
            Checkpoints::<Runtime>::insert(SUPPORTED_CHAIN_KEY, valid_height, valid_digest);

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::find_highest_attested_before {
                        chain_key: SUPPORTED_CHAIN_KEY,
                        target_height,
                    },
                )
                .execute_returns(expected_result);
        });
}

// Bugbot regression (PR #1108) for the same downward-walk bound in `is_height_attested`: last
// checkpoint one block above target, a valid checkpoint ~8 buckets below — must report attested.
#[test]
fn is_height_attested_finds_far_checkpoint_when_last_checkpoint_just_above_target() {
    let alice: H160 = Alice.into();

    let target_height: u64 = 10_000;
    let below_height: u64 = 2_000; // 8 buckets below target pivot
    let last_checkpoint_height: u64 = 10_001; // just above target => tiny abs_diff

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            LastCheckpoint::<Runtime>::insert(
                SUPPORTED_CHAIN_KEY,
                AttestationCheckpoint {
                    block_number: last_checkpoint_height,
                    digest: H256::random(),
                },
            );

            let below_pivot = AttestationPallet::<Runtime>::compute_block_index_for(below_height);
            CheckpointBuckets::<Runtime>::insert(
                (SUPPORTED_CHAIN_KEY, below_pivot, below_height),
                (),
            );
            Checkpoints::<Runtime>::insert(
                SUPPORTED_CHAIN_KEY,
                below_height,
                H256::from_slice(&[88_u8; 32]),
            );

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::is_height_attested {
                        chain_key: SUPPORTED_CHAIN_KEY,
                        target_height,
                    },
                )
                .execute_returns(true);
        });
}

// Bugbot regression (PR #1108, 2nd pass): the bucket walk must not be clamped to a fixed ceiling.
// The earlier fix capped attempts at MAX_BUCKET_SEARCH_ATTEMPTS=4096, so for a target above
// ~4_096_000 blocks the downward walk stopped thousands of buckets short of genesis and
// false-negatived. Here target is ~4.098M with a valid checkpoint 4097 buckets below (beyond the
// old ceiling): it must still be found now that the walk is bounded by gas, not a fixed cap.
#[test]
fn find_highest_attested_before_finds_checkpoint_beyond_old_fixed_ceiling() {
    let alice: H160 = Alice.into();

    // 4097 buckets below the target pivot => beyond the old 4096-attempt clamp.
    let target_height: u64 = 4_098_000; // pivot 4_098_000
    let valid_height: u64 = 1_000; // pivot 1_000
    let valid_digest = H256::from_slice(&[99_u8; 32]);
    let last_checkpoint_height: u64 = 4_098_500; // >= target, forces the bucket-search branch

    let expected_result = HeightHashResult {
        height: valid_height,
        hash: valid_digest,
        is_attestation: false,
        exists: true,
    };

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            LastCheckpoint::<Runtime>::insert(
                SUPPORTED_CHAIN_KEY,
                AttestationCheckpoint {
                    block_number: last_checkpoint_height,
                    digest: H256::random(),
                },
            );

            let valid_pivot = AttestationPallet::<Runtime>::compute_block_index_for(valid_height);
            CheckpointBuckets::<Runtime>::insert(
                (SUPPORTED_CHAIN_KEY, valid_pivot, valid_height),
                (),
            );
            Checkpoints::<Runtime>::insert(SUPPORTED_CHAIN_KEY, valid_height, valid_digest);

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::find_highest_attested_before {
                        chain_key: SUPPORTED_CHAIN_KEY,
                        target_height,
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

/// Regression: during the async post-revert pruning window `Checkpoints` still holds stale
/// digests for buckets at or above `CheckpointPruningStates.next_pivot`. Consensus-side
/// `block-prover` already gates by this state via `checkpoint_if_stable`; the informational
/// chain-info precompile now does the same so it can't surface a stale digest as live metadata.
mod checkpoint_reads_honour_pruning_window {
    use super::*;
    use pallet_attestation::{
        clear_or_revert::CheckpointPruningState, CheckpointPruningStates, CHECKPOINT_BUCKET_SIZE,
    };

    /// Pick a height whose bucket pivot is strictly above `LastCheckpoint.block_number` and
    /// equal to the test's `next_pivot`, so the pruning state gates it.
    fn in_flight_height() -> u64 {
        // 2 * BUCKET_SIZE puts us in the bucket immediately above checkpoint_pivot=1*BUCKET_SIZE.
        2 * CHECKPOINT_BUCKET_SIZE
    }

    fn install_pruning_state(stop_height: u64, next_pivot: u64) {
        CheckpointPruningStates::<Runtime>::insert(
            SUPPORTED_CHAIN_KEY,
            CheckpointPruningState {
                stop_height,
                next_pivot,
            },
        );
    }

    #[test]
    fn get_checkpoint_for_height_returns_default_during_pruning() {
        let alice: H160 = Alice.into();
        let stale_height = in_flight_height();
        let stale_digest = H256::from_slice(&[0x9c; 32]);

        ExtBuilder::default()
            .with_balances(vec![(alice.into(), 300)])
            .build()
            .execute_with(|| {
                Checkpoints::<Runtime>::insert(SUPPORTED_CHAIN_KEY, stale_height, stale_digest);
                install_pruning_state(CHECKPOINT_BUCKET_SIZE, in_flight_height());

                precompiles()
                    .prepare_test(
                        alice,
                        Precompile,
                        PCall::get_checkpoint_for_height {
                            chain_key: SUPPORTED_CHAIN_KEY,
                            height: stale_height,
                        },
                    )
                    .execute_returns(HashResult::default());
            });
    }

    #[test]
    fn get_checkpoint_for_height_returns_digest_below_pruning_pivot() {
        let alice: H160 = Alice.into();
        // A height in a bucket strictly below `next_pivot` is stable (synchronously cleaned
        // inside `do_revert_to` or never touched by the revert), so reads should pass through.
        let stable_height = CHECKPOINT_BUCKET_SIZE / 2;
        let stable_digest = H256::from_slice(&[0x4d; 32]);

        ExtBuilder::default()
            .with_balances(vec![(alice.into(), 300)])
            .build()
            .execute_with(|| {
                Checkpoints::<Runtime>::insert(SUPPORTED_CHAIN_KEY, stable_height, stable_digest);
                install_pruning_state(CHECKPOINT_BUCKET_SIZE, in_flight_height());

                precompiles()
                    .prepare_test(
                        alice,
                        Precompile,
                        PCall::get_checkpoint_for_height {
                            chain_key: SUPPORTED_CHAIN_KEY,
                            height: stable_height,
                        },
                    )
                    .execute_returns(HashResult {
                        hash: stable_digest,
                        exists: true,
                    });
            });
    }

    #[test]
    fn get_checkpoint_for_height_returns_digest_when_no_pruning_state() {
        let alice: H160 = Alice.into();
        // No `CheckpointPruningStates` entry → no revert in flight → reads pass through
        // unconditionally, mirroring `checkpoint_if_stable`'s fast path.
        let height = in_flight_height();
        let digest = H256::from_slice(&[0x77; 32]);

        ExtBuilder::default()
            .with_balances(vec![(alice.into(), 300)])
            .build()
            .execute_with(|| {
                Checkpoints::<Runtime>::insert(SUPPORTED_CHAIN_KEY, height, digest);

                precompiles()
                    .prepare_test(
                        alice,
                        Precompile,
                        PCall::get_checkpoint_for_height {
                            chain_key: SUPPORTED_CHAIN_KEY,
                            height,
                        },
                    )
                    .execute_returns(HashResult {
                        hash: digest,
                        exists: true,
                    });
            });
    }

    /// Regression: `find_highest_attested_before`'s fallback path falls through to
    /// `LastCheckpoint` when no attestation matches. If the last checkpoint's height is in a
    /// pivot still being drained post-revert, `checkpoint_if_stable` returns `None` — the
    /// fallback must surface that as `exists: false` rather than advertising a zero digest
    /// with `exists: true`, matching `get_checkpoint_for_height`'s gated behaviour.
    #[test]
    fn find_highest_attested_before_fallback_doesnt_advertise_withheld_digest() {
        let alice: H160 = Alice.into();
        // No attestations populated, so the precompile will land in the LastCheckpoint
        // fallback. Pick LastCheckpoint at an in-flight pivot and a `target_height` ABOVE it
        // so the outer gate `LastCheckpoint >= target_height` fails and we reach the else
        // branch holding the fallback.
        let last_checkpoint_height = in_flight_height();
        let stale_digest = H256::from_slice(&[0xfe; 32]);
        let target_height = last_checkpoint_height + 1;

        ExtBuilder::default()
            .with_balances(vec![(alice.into(), 300)])
            .build()
            .execute_with(|| {
                Checkpoints::<Runtime>::insert(
                    SUPPORTED_CHAIN_KEY,
                    last_checkpoint_height,
                    stale_digest,
                );
                LastCheckpoint::<Runtime>::set(
                    SUPPORTED_CHAIN_KEY,
                    Some(AttestationCheckpoint {
                        block_number: last_checkpoint_height,
                        digest: stale_digest,
                    }),
                );
                install_pruning_state(CHECKPOINT_BUCKET_SIZE, in_flight_height());

                precompiles()
                    .prepare_test(
                        alice,
                        Precompile,
                        PCall::find_highest_attested_before {
                            chain_key: SUPPORTED_CHAIN_KEY,
                            target_height,
                        },
                    )
                    .execute_returns(HeightHashResult::default());
            });
    }
}

/// Each public method that takes a `chain_key` reverts when the chain isn't in
/// `SupportedChains`. Reverting (rather than returning a zero default) prevents Solidity
/// callers from confusing "no data yet" with "bad input", and refunds remaining gas instead
/// of burning the rest of the method's lookups.
mod unsupported_chain_reverts {
    use super::*;

    const UNSUPPORTED_CHAIN_KEY: u64 = 999;
    const EXPECTED_REVERT: &[u8] = b"chain not supported";

    #[test]
    fn get_attestation_genesis_height_reverts() {
        let alice: H160 = Alice.into();
        ExtBuilder::default()
            .with_balances(vec![(alice.into(), 300)])
            .build()
            .execute_with(|| {
                precompiles()
                    .prepare_test(
                        alice,
                        Precompile,
                        PCall::get_attestation_genesis_height {
                            chain_key: UNSUPPORTED_CHAIN_KEY,
                        },
                    )
                    .execute_reverts(|out| out == EXPECTED_REVERT);
            });
    }

    #[test]
    fn get_latest_attestation_height_and_hash_reverts() {
        let alice: H160 = Alice.into();
        ExtBuilder::default()
            .with_balances(vec![(alice.into(), 300)])
            .build()
            .execute_with(|| {
                precompiles()
                    .prepare_test(
                        alice,
                        Precompile,
                        PCall::get_latest_attestation_height_and_hash {
                            chain_key: UNSUPPORTED_CHAIN_KEY,
                        },
                    )
                    .execute_reverts(|out| out == EXPECTED_REVERT);
            });
    }

    #[test]
    fn get_latest_checkpoint_height_and_hash_reverts() {
        let alice: H160 = Alice.into();
        ExtBuilder::default()
            .with_balances(vec![(alice.into(), 300)])
            .build()
            .execute_with(|| {
                precompiles()
                    .prepare_test(
                        alice,
                        Precompile,
                        PCall::get_latest_checkpoint_height_and_hash {
                            chain_key: UNSUPPORTED_CHAIN_KEY,
                        },
                    )
                    .execute_reverts(|out| out == EXPECTED_REVERT);
            });
    }

    #[test]
    fn find_highest_attested_before_reverts() {
        let alice: H160 = Alice.into();
        ExtBuilder::default()
            .with_balances(vec![(alice.into(), 300)])
            .build()
            .execute_with(|| {
                precompiles()
                    .prepare_test(
                        alice,
                        Precompile,
                        PCall::find_highest_attested_before {
                            chain_key: UNSUPPORTED_CHAIN_KEY,
                            target_height: 100,
                        },
                    )
                    .execute_reverts(|out| out == EXPECTED_REVERT);
            });
    }

    #[test]
    fn find_lowest_attested_after_reverts() {
        let alice: H160 = Alice.into();
        ExtBuilder::default()
            .with_balances(vec![(alice.into(), 300)])
            .build()
            .execute_with(|| {
                precompiles()
                    .prepare_test(
                        alice,
                        Precompile,
                        PCall::find_lowest_attested_after {
                            chain_key: UNSUPPORTED_CHAIN_KEY,
                            target_height: 100,
                        },
                    )
                    .execute_reverts(|out| out == EXPECTED_REVERT);
            });
    }

    #[test]
    fn is_height_attested_reverts() {
        let alice: H160 = Alice.into();
        ExtBuilder::default()
            .with_balances(vec![(alice.into(), 300)])
            .build()
            .execute_with(|| {
                precompiles()
                    .prepare_test(
                        alice,
                        Precompile,
                        PCall::is_height_attested {
                            chain_key: UNSUPPORTED_CHAIN_KEY,
                            target_height: 100,
                        },
                    )
                    .execute_reverts(|out| out == EXPECTED_REVERT);
            });
    }

    #[test]
    fn get_attestation_bounds_reverts() {
        let alice: H160 = Alice.into();
        ExtBuilder::default()
            .with_balances(vec![(alice.into(), 300)])
            .build()
            .execute_with(|| {
                precompiles()
                    .prepare_test(
                        alice,
                        Precompile,
                        PCall::get_attestation_bounds {
                            chain_key: UNSUPPORTED_CHAIN_KEY,
                            target_height: 100,
                        },
                    )
                    .execute_reverts(|out| out == EXPECTED_REVERT);
            });
    }

    #[test]
    fn get_attestation_height_for_digest_reverts() {
        let alice: H160 = Alice.into();
        ExtBuilder::default()
            .with_balances(vec![(alice.into(), 300)])
            .build()
            .execute_with(|| {
                precompiles()
                    .prepare_test(
                        alice,
                        Precompile,
                        PCall::get_attestation_height_for_digest {
                            chain_key: UNSUPPORTED_CHAIN_KEY,
                            digest: H256::from_slice(&[0xab; 32]),
                        },
                    )
                    .execute_reverts(|out| out == EXPECTED_REVERT);
            });
    }

    #[test]
    fn get_checkpoint_for_height_reverts() {
        let alice: H160 = Alice.into();
        ExtBuilder::default()
            .with_balances(vec![(alice.into(), 300)])
            .build()
            .execute_with(|| {
                precompiles()
                    .prepare_test(
                        alice,
                        Precompile,
                        PCall::get_checkpoint_for_height {
                            chain_key: UNSUPPORTED_CHAIN_KEY,
                            height: 100,
                        },
                    )
                    .execute_reverts(|out| out == EXPECTED_REVERT);
            });
    }
}
