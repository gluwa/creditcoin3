//! Regression tests for the batch-verify heap-exhaustion vector.
//!
//! The precompile ABI-decodes every argument onto the runtime heap before any entry point
//! runs, and the decoder does not reject aliased ABI offsets, so a small call could decode
//! one region many times into large allocations and exhaust the import heap (halting the
//! chain). The fix charges for the raw input first and bounds every allocating argument at
//! decode time:
//!   * `encoded_transaction(s)` -> `BoundedBytes<MaxEncodedTxBytes>` (4 MiB each),
//!   * a batch's `encoded_transactions` -> `BatchEncodedTxs` (`MaxBatchBytes`, 4 MiB in total),
//!   * Merkle proof `siblings`  -> `BoundedVec<_, 64>` (in `TransactionMerkleProof::read`),
//!   * continuity `roots`       -> `BoundedVec<_, MAX_CONTINUITY_ROOTS>` (in `ContinuityProof::read`).
//!
//! These tests drive the real decode path (the harness ABI-encodes the call, the precompile
//! decodes it) and assert that an over-limit argument reverts *during decode* with
//! "Value is too large", i.e. before the oversized buffer is ever allocated.

use crate::mock::*;
use crate::tests::precompiles;
use crate::*;
use attestor_primitives::block::ContinuityProof;
use merkle::{MerkleProofEntry, TransactionMerkleProof};
use precompile_utils::testing::*;

/// True when the revert output carries the decoder's over-limit message.
fn is_too_large(output: &[u8]) -> bool {
    String::from_utf8_lossy(output).contains("Value is too large")
}

fn small_merkle_proof() -> TransactionMerkleProof {
    TransactionMerkleProof {
        root: sp_core::H256::zero(),
        siblings: vec![MerkleProofEntry {
            hash: sp_core::H256::zero(),
            is_left: false,
        }],
    }
}

fn small_continuity_proof() -> ContinuityProof {
    ContinuityProof::new(sp_core::H256::zero(), vec![sp_core::H256::zero()])
}

#[test]
fn verify_rejects_oversized_encoded_transaction_at_decode() {
    ExtBuilder::default().build().execute_with(|| {
        // One byte over the MaxEncodedTxBytes cap.
        let tx_data = vec![0u8; MAX_TX + 1];
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: 1,
                    height: 2,
                    encoded_transaction: tx_data.into(),
                    merkle_proof: small_merkle_proof(),
                    continuity_proof: small_continuity_proof(),
                },
            )
            .execute_reverts(is_too_large);
    });
}

#[test]
fn verify_rejects_too_many_merkle_siblings_at_decode() {
    ExtBuilder::default().build().execute_with(|| {
        // 65 siblings: one past the 64-deep cap (MAX_MERKLE_SIBLINGS).
        let siblings = vec![
            MerkleProofEntry {
                hash: sp_core::H256::zero(),
                is_left: false
            };
            65
        ];
        let merkle_proof = TransactionMerkleProof {
            root: sp_core::H256::zero(),
            siblings,
        };
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: 1,
                    height: 2,
                    encoded_transaction: vec![1u8].into(),
                    merkle_proof,
                    continuity_proof: small_continuity_proof(),
                },
            )
            .execute_reverts(is_too_large);
    });
}

#[test]
fn verify_rejects_too_many_continuity_roots_at_decode() {
    ExtBuilder::default().build().execute_with(|| {
        let roots = vec![sp_core::H256::zero(); crate::verify::MAX_CONTINUITY_ROOTS + 1];
        let continuity_proof = ContinuityProof::new(sp_core::H256::zero(), roots);
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: 1,
                    height: 2,
                    encoded_transaction: vec![1u8].into(),
                    merkle_proof: small_merkle_proof(),
                    continuity_proof,
                },
            )
            .execute_reverts(is_too_large);
    });
}

#[test]
fn verify_batch_rejects_oversized_arguments_at_decode() {
    ExtBuilder::default().build().execute_with(|| {
        // A batch call whose one Merkle proof is over-deep: rejected at decode, before the
        // array-length-mismatch check (the audit PoC relied on decode happening first).
        let over_deep = TransactionMerkleProof {
            root: sp_core::H256::zero(),
            siblings: vec![
                MerkleProofEntry {
                    hash: sp_core::H256::zero(),
                    is_left: false
                };
                65
            ],
        };
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_batch {
                    chain_key: 1,
                    heights: vec![2u64].into(),
                    encoded_transactions: vec![vec![1u8].into()].into(),
                    merkle_proofs: vec![over_deep].into(),
                    shared_continuity_proof: small_continuity_proof(),
                },
            )
            .execute_reverts(is_too_large);
    });
}

// ---------------------------------------------------------------------------------------
// Aliased ABI offsets and pre-decode gas
// ---------------------------------------------------------------------------------------

const BATCH: usize = 10;
const MIB: usize = 1_048_576;
/// `MaxEncodedTxBytes`.
const MAX_TX: usize = 4 * MIB;
/// `MaxBatchBytes`.
const MAX_BATCH: usize = 4 * MIB;

#[test]
fn caps_match_the_runtime_types() {
    use sp_core::Get;
    assert_eq!(<MaxEncodedTxBytes as Get<u32>>::get() as usize, MAX_TX);
    assert_eq!(<MaxBatchBytes as Get<u32>>::get() as usize, MAX_BATCH);
}

fn word_at(args: &[u8], pos: usize) -> usize {
    sp_core::U256::from_big_endian(&args[pos..pos + 32]).as_usize()
}

/// Raw batch-`verify` input whose `BATCH` `encoded_transactions` entries all point at one
/// `tx_len`-byte region — the audit PoC shape. The honest encoder cannot produce aliased
/// offsets, so the call is encoded normally and the offsets are then rewritten.
fn aliased_batch_input(tx_len: usize) -> Vec<u8> {
    let mut txs = vec![vec![1u8]; BATCH];
    txs[0] = vec![0x55u8; tx_len];

    let mut input = PCall::verify_batch {
        chain_key: 1,
        heights: vec![2u64; BATCH].into(),
        encoded_transactions: txs.into_iter().map(Into::into).collect::<Vec<_>>().into(),
        merkle_proofs: vec![small_merkle_proof(); BATCH].into(),
        shared_continuity_proof: small_continuity_proof(),
    }
    .encode();
    assert_eq!(
        input[..4],
        [0x1b, 0x5f, 0x6f, 0x88],
        "batch verify selector"
    );

    // Head words: chain_key, heights offset, encoded_transactions offset, ...
    let args = &mut input[4..];
    let array = word_at(args, 2 * 32);
    assert_eq!(word_at(args, array), BATCH);
    let first_offset = array + 32;
    for i in 1..BATCH {
        args.copy_within(first_offset..first_offset + 32, first_offset + 32 * i);
    }
    input
}

fn input_decode_gas(input: &[u8]) -> u64 {
    crate::verify::INPUT_DECODE_WORD_COST * crate::verify::words(input.len())
}

fn batch_call(txs: Vec<Vec<u8>>) -> PCall {
    let n = txs.len();
    PCall::verify_batch {
        chain_key: 1,
        heights: vec![2u64; n].into(),
        encoded_transactions: txs.into_iter().map(Into::into).collect::<Vec<_>>().into(),
        merkle_proofs: vec![small_merkle_proof(); n].into(),
        shared_continuity_proof: small_continuity_proof(),
    }
}

#[test]
fn aliased_offsets_are_rejected_at_decode() {
    ExtBuilder::default().build().execute_with(|| {
        // The audit PoC shape: ten offsets at one region. Rejected whether the region is at
        // the per-item cap (40 MiB decoded) or small enough that only the total is over
        // (10 x 512 KiB = 5 MiB).
        for tx_len in [MAX_TX, 3 * MIB, MIB / 2] {
            precompiles()
                .prepare_test(
                    Account::Alice,
                    Account::Precompile,
                    aliased_batch_input(tx_len),
                )
                .execute_reverts(is_too_large);
        }
    });
}

#[test]
fn aliased_offsets_within_the_batch_cap_decode_and_fail_normally() {
    ExtBuilder::default().build().execute_with(|| {
        // Ten aliases of a 256 KiB region decode to 2.5 MiB: under the batch cap, so the call
        // proceeds to the ordinary verification failure instead of a decode error.
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                aliased_batch_input(MIB / 4),
            )
            .execute_reverts(|output| !is_too_large(output));
    });
}

#[test]
fn honest_batch_over_the_batch_cap_is_rejected_at_decode() {
    ExtBuilder::default().build().execute_with(|| {
        // Three distinct 2 MiB transactions: each is under the per-item cap, the batch is not.
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                batch_call(vec![vec![0x55u8; 2 * MIB]; 3]),
            )
            .execute_reverts(is_too_large);
    });
}

#[test]
fn batch_cap_counts_power_of_two_rounded_lengths() {
    ExtBuilder::default().build().execute_with(|| {
        // Two 2 MiB transactions total exactly the batch cap and decode.
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                batch_call(vec![vec![0x55u8; 2 * MIB]; 2]),
            )
            .execute_reverts(|output| !is_too_large(output));

        // Empty items allocate nothing, so they do not count towards the cap.
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                batch_call(vec![vec![0x55u8; MAX_BATCH], vec![], vec![]]),
            )
            .execute_reverts(|output| !is_too_large(output));

        // One byte more on one of them rounds it up to 4 MiB, taking the batch over.
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                batch_call(vec![vec![0x55u8; 2 * MIB + 1], vec![0x55u8; 2 * MIB]]),
            )
            .execute_reverts(is_too_large);
    });
}

#[test]
fn batch_rejects_oversized_item_and_too_many_items_at_decode() {
    ExtBuilder::default().build().execute_with(|| {
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                batch_call(vec![vec![0u8; MAX_TX + 1]]),
            )
            .execute_reverts(is_too_large);

        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                batch_call(vec![vec![1u8]; BATCH + 1]),
            )
            .execute_reverts(is_too_large);
    });
}

#[test]
fn batch_encoded_txs_round_trips_as_bytes_array() {
    use precompile_utils::solidity::codec::{Reader, Writer};
    use precompile_utils::solidity::Codec;

    assert_eq!(<BatchEncodedTxs as Codec>::signature(), "bytes[]");
    assert!(!<BatchEncodedTxs as Codec>::has_static_size());

    // Decodes exactly what the stock `bytes[]` encoder writes, and encodes the same bytes.
    let txs: Vec<BoundedBytes<MaxEncodedTxBytes>> = vec![
        vec![1u8, 2, 3].into(),
        vec![].into(),
        vec![0xabu8; 100].into(),
    ];
    let encoded = Writer::new().write(txs.clone()).build();
    let decoded: BatchEncodedTxs = Reader::new(&encoded).read().expect("decodes");
    assert_eq!(Vec::from(decoded.clone()), txs);
    assert_eq!(Writer::new().write(decoded).build(), encoded);
}

#[test]
fn input_is_charged_before_decode() {
    ExtBuilder::default().build().execute_with(|| {
        // One gas short of the input charge: the call runs out of gas in the pre-check, so
        // the aliased arguments are never decoded.
        let input = aliased_batch_input(MIB);
        let decode_gas = input_decode_gas(&input);
        precompiles()
            .prepare_test(Account::Alice, Account::Precompile, input)
            .with_target_gas(Some(decode_gas - 1))
            .execute_error(ExitError::OutOfGas);
    });
}

#[test]
fn leaf_hash_is_charged_by_transaction_size() {
    ExtBuilder::default().build().execute_with(|| {
        use crate::verify::{
            words, CONTINUITY_BLOCK_HASH_COST, KECCAK_BASE_COST, KECCAK_WORD_COST,
        };

        // A 1 MiB transaction with a one-sibling proof that does not verify: the call pays
        // for the input, the leaf hash and the sibling hash, then reverts.
        let tx_len = 1_048_576;
        let input = PCall::verify {
            chain_key: 1,
            height: 2,
            encoded_transaction: vec![0x55u8; tx_len].into(),
            merkle_proof: small_merkle_proof(),
            continuity_proof: small_continuity_proof(),
        }
        .encode();
        let expected = input_decode_gas(&input)
            + KECCAK_BASE_COST
            + KECCAK_WORD_COST * words(tx_len + 1)
            + CONTINUITY_BLOCK_HASH_COST;

        precompiles()
            .prepare_test(Account::Alice, Account::Precompile, input)
            .expect_cost(expected)
            .execute_reverts(|output| output.ends_with(b"Merkle proof validation failed"));
    });
}

// ---------------------------------------------------------------------------------------
// Differential fuzz: `BatchEncodedTxs` against the stock `bytes[]` decoder
// ---------------------------------------------------------------------------------------

/// xorshift64*: deterministic, so a failure reproduces from the printed case number.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

type StockBatch = BoundedVec<BoundedBytes<MaxEncodedTxBytes>, MaxBatchSize>;

/// Decode `input` with both decoders and check they agree wherever the batch cap allows.
fn check_against_stock(case: usize, input: &[u8]) {
    use precompile_utils::solidity::codec::Reader;

    let stock: Result<Vec<BoundedBytes<MaxEncodedTxBytes>>, _> =
        Reader::new(input).read::<StockBatch>().map(Into::into);
    let ours: Result<Vec<BoundedBytes<MaxEncodedTxBytes>>, _> =
        Reader::new(input).read::<BatchEncodedTxs>().map(Into::into);

    match (stock, ours) {
        (Ok(stock), Ok(ours)) => assert_eq!(stock, ours, "case {case}: decoded values differ"),
        (Ok(stock), Err(_)) => {
            let total: usize = stock
                .iter()
                .map(|tx| tx.as_bytes().len())
                .filter(|len| *len > 0)
                .map(usize::next_power_of_two)
                .sum();
            assert!(
                total > MAX_BATCH,
                "case {case}: rejected a batch the stock decoder accepts, total {total}"
            );
        }
        (Err(_), Ok(_)) => panic!("case {case}: accepted an input the stock decoder rejects"),
        (Err(_), Err(_)) => {}
    }
}

#[test]
fn batch_decoder_matches_stock_decoder_on_fuzzed_input() {
    use precompile_utils::solidity::codec::Writer;

    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut accepted = 0usize;
    let mut over_cap = 0usize;

    for case in 0..3_000 {
        // An honest encoding: up to 12 items (the batch limit is 10), mostly small, with
        // every 50th case large enough for aliasing to reach the batch cap.
        let large = case % 50 == 0;
        let items = rng.below(13);
        let txs: Vec<BoundedBytes<MaxEncodedTxBytes>> = (0..items)
            .map(|_| {
                let len = match rng.below(4) {
                    0 => 0,
                    1 => rng.below(64),
                    _ if large => rng.below(MIB + MIB / 2),
                    _ => rng.below(2_000),
                };
                vec![rng.next() as u8; len].into()
            })
            .collect();
        let mut input = Writer::new().write(txs).build();
        check_against_stock(case, &input);

        // Mutations: rewrite whole ABI words (offsets and lengths live on word boundaries) so
        // offsets alias, point backwards or out of bounds, and lengths lie; then truncate.
        let words = input.len() / 32;
        for _ in 0..rng.below(4) {
            let at = 32 * rng.below(words);
            let value: u64 = match rng.below(5) {
                0 => 0,
                1 => 32 * rng.below(words) as u64,
                2 => rng.below(input.len() + 64) as u64,
                3 => rng.next() >> rng.below(64),
                // Copy another word, which is how offsets come to alias one region.
                _ => {
                    let from = 32 * rng.below(words);
                    input.copy_within(from..from + 32, at);
                    continue;
                }
            };
            input[at..at + 32].copy_from_slice(&sp_core::U256::from(value).to_big_endian());
        }
        if rng.below(8) == 0 {
            input.truncate(rng.below(input.len() + 1));
        }
        check_against_stock(case, &input);

        use precompile_utils::solidity::codec::Reader;
        match Reader::new(&input).read::<BatchEncodedTxs>() {
            Ok(_) => accepted += 1,
            Err(_) if Reader::new(&input).read::<StockBatch>().is_ok() => over_cap += 1,
            Err(_) => {}
        }
    }

    // The corpus must exercise both outcomes, or the comparison above proves nothing.
    assert!(accepted > 500, "only {accepted} mutated inputs decoded");
    assert!(
        over_cap > 0,
        "no mutated input was rejected by the batch cap alone"
    );
}
