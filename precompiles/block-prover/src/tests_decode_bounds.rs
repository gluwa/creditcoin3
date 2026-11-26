//! Tests for the size limits and gas charges the precompile applies to its input.
//!
//! Each allocating argument has a bound that is checked while the call is decoded:
//!   * `encoded_transaction(s)` -> `BoundedBytes<MaxEncodedTxBytes>` (4 MiB each),
//!   * a batch's `encoded_transactions` -> `BatchEncodedTxs` (`MaxBatchBytes`, 4 MiB in total),
//!   * Merkle proof `siblings`  -> `BoundedVec<_, 64>` (in `TransactionMerkleProof::read`),
//!   * continuity `roots`       -> `BoundedVec<_, MAX_CONTINUITY_ROOTS>` (in `ContinuityProof::read`).
//!
//! These tests drive the real decode path (the harness ABI-encodes the call, the precompile
//! decodes it). An over-limit argument must revert with that argument's own decode error; an
//! input within the limits must get past decode and fail verification instead.

use crate::mock::*;
use crate::tests::precompiles;
use crate::*;
use attestor_primitives::block::ContinuityProof;
use merkle::{MerkleProofEntry, TransactionMerkleProof};
use precompile_utils::testing::*;

const BATCH: usize = 10;
const MIB: usize = 1_048_576;
/// `MaxEncodedTxBytes`.
const MAX_TX: usize = 4 * MIB;
/// `MaxBatchBytes`.
const MAX_BATCH: usize = 4 * MIB;
/// `MAX_MERKLE_SIBLINGS`.
const MAX_SIBLINGS: usize = 64;

/// What a batch call reverts with once it has decoded: the mock has no attestation for the
/// proof.
const DECODED: &str = "Continuity proof does not match attestation or checkpoint";

/// Matches a revert whose output is exactly `expected`.
fn reverts_with(expected: &str) -> impl Fn(&[u8]) -> bool + '_ {
    move |output| {
        if output != expected.as_bytes() {
            eprintln!(
                "expected revert {expected:?}, got {:?}",
                String::from_utf8_lossy(output)
            );
        }
        output == expected.as_bytes()
    }
}

/// The revert for batch item `i` taking the batch over `MaxBatchBytes`.
fn over_batch_cap(i: usize) -> String {
    format!("encodedTransactions[{i}]: Value is too large for batch length")
}

fn merkle_proof_with(siblings: usize) -> TransactionMerkleProof {
    TransactionMerkleProof {
        root: sp_core::H256::zero(),
        siblings: vec![
            MerkleProofEntry {
                hash: sp_core::H256::zero(),
                is_left: false
            };
            siblings
        ],
    }
}

fn small_merkle_proof() -> TransactionMerkleProof {
    merkle_proof_with(1)
}

fn small_continuity_proof() -> ContinuityProof {
    ContinuityProof::new(sp_core::H256::zero(), vec![sp_core::H256::zero()])
}

#[test]
fn caps_match_the_runtime_types() {
    use sp_core::Get;
    assert_eq!(<MaxEncodedTxBytes as Get<u32>>::get() as usize, MAX_TX);
    assert_eq!(<MaxBatchBytes as Get<u32>>::get() as usize, MAX_BATCH);
    assert_eq!(<MaxBatchSize as Get<u32>>::get() as usize, BATCH);
    assert_eq!(merkle::proof::MAX_MERKLE_SIBLINGS as usize, MAX_SIBLINGS);
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
            .execute_reverts(reverts_with(
                "encodedTransaction: Value is too large for length",
            ));
    });
}

#[test]
fn verify_bounds_merkle_siblings_at_decode() {
    ExtBuilder::default().build().execute_with(|| {
        let call = |siblings| PCall::verify {
            chain_key: 1,
            height: 2,
            encoded_transaction: vec![1u8].into(),
            merkle_proof: merkle_proof_with(siblings),
            continuity_proof: small_continuity_proof(),
        };

        // Exactly at the cap: decodes, and the proof is then checked.
        precompiles()
            .prepare_test(Account::Alice, Account::Precompile, call(MAX_SIBLINGS))
            .execute_reverts(reverts_with("Merkle proof validation failed"));

        // One past it.
        precompiles()
            .prepare_test(Account::Alice, Account::Precompile, call(MAX_SIBLINGS + 1))
            .execute_reverts(reverts_with(
                "merkleProof.siblings: Value is too large for length",
            ));
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
            .execute_reverts(reverts_with(
                "continuityProof.roots: Value is too large for length",
            ));
    });
}

#[test]
fn verify_batch_rejects_oversized_arguments_before_the_length_check() {
    ExtBuilder::default().build().execute_with(|| {
        // Two heights against one transaction and one proof is a length mismatch, and the
        // proof is over-deep. The decode error is the one reported: arguments are decoded,
        // and bounded, before the entry point compares the array lengths.
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_batch {
                    chain_key: 1,
                    heights: vec![2u64, 3u64].into(),
                    encoded_transactions: vec![vec![1u8].into()].into(),
                    merkle_proofs: vec![merkle_proof_with(MAX_SIBLINGS + 1)].into(),
                    shared_continuity_proof: small_continuity_proof(),
                },
            )
            .execute_reverts(reverts_with(
                "merkleProofs[0].siblings: Value is too large for length",
            ));
    });
}

// ---------------------------------------------------------------------------------------
// The batch total, and the charge for the raw input
// ---------------------------------------------------------------------------------------

fn word_at(args: &[u8], pos: usize) -> usize {
    sp_core::U256::from_big_endian(&args[pos..pos + 32]).as_usize()
}

/// Raw batch-`verify` input whose `BATCH` `encoded_transactions` offsets all point at the
/// same `tx_len`-byte item. The encoder never writes repeated offsets, so the call is
/// encoded normally and the offsets are then rewritten.
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
        PCall::verify_batch_selectors()[0].to_be_bytes(),
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
fn repeated_offsets_count_towards_the_batch_cap() {
    ExtBuilder::default().build().execute_with(|| {
        // Every offset is counted, whether or not it points at an item already seen. The
        // index in the error is the item that takes the running total past the cap:
        //   4 MiB   -> the second item,
        //   3 MiB   -> counts as 4 MiB, so also the second,
        //   512 KiB -> the ninth (4.5 MiB).
        for (tx_len, first_over) in [(MAX_TX, 1), (3 * MIB, 1), (MIB / 2, 8)] {
            precompiles()
                .prepare_test(
                    Account::Alice,
                    Account::Precompile,
                    aliased_batch_input(tx_len),
                )
                .execute_reverts(reverts_with(&over_batch_cap(first_over)));
        }
    });
}

#[test]
fn repeated_offsets_within_the_batch_cap_decode() {
    ExtBuilder::default().build().execute_with(|| {
        // Ten offsets at one 256 KiB item count as 2.5 MiB: under the batch cap.
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                aliased_batch_input(MIB / 4),
            )
            .execute_reverts(reverts_with(DECODED));
    });
}

#[test]
fn batch_over_the_batch_cap_is_rejected_at_decode() {
    ExtBuilder::default().build().execute_with(|| {
        // Three distinct 2 MiB transactions: each is under the per-item cap, the batch is not.
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                batch_call(vec![vec![0x55u8; 2 * MIB]; 3]),
            )
            .execute_reverts(reverts_with(&over_batch_cap(2)));
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
            .execute_reverts(reverts_with(DECODED));

        // Empty items allocate nothing, so they do not count towards the cap.
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                batch_call(vec![vec![0x55u8; MAX_BATCH], vec![], vec![]]),
            )
            .execute_reverts(reverts_with(DECODED));

        // 3 MiB + 1 of bytes, which is under the cap; 2 MiB + 1 counts as 4 MiB, so the
        // second item takes the total to 5 MiB.
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                batch_call(vec![vec![0x55u8; 2 * MIB + 1], vec![0x55u8; MIB]]),
            )
            .execute_reverts(reverts_with(&over_batch_cap(1)));

        // 3 MiB + 3 of bytes; each item counts as 2 MiB, so the third takes it to 6 MiB.
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                batch_call(vec![vec![0x55u8; MIB + 1]; 3]),
            )
            .execute_reverts(reverts_with(&over_batch_cap(2)));
    });
}

#[test]
fn batch_rejects_an_oversized_item_at_decode() {
    ExtBuilder::default().build().execute_with(|| {
        // The per-item cap, not the batch total: a different error.
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                batch_call(vec![vec![0u8; MAX_TX + 1]]),
            )
            .execute_reverts(reverts_with(
                "encodedTransactions[0]: Value is too large for length",
            ));
    });
}

#[test]
fn batch_decoder_rejects_too_many_items() {
    use precompile_utils::solidity::codec::{Reader, Writer};

    // At the codec level: in a full call `heights` decodes first and reports the same limit.
    let encode = |items| {
        Writer::new()
            .write(vec![
                BoundedBytes::<MaxEncodedTxBytes>::from(vec![1u8]);
                items
            ])
            .build()
    };
    assert!(Reader::new(&encode(BATCH))
        .read::<BatchEncodedTxs>()
        .is_ok());
    assert!(Reader::new(&encode(BATCH + 1))
        .read::<BatchEncodedTxs>()
        .is_err());
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
        // Ten offsets at one 1 MiB item: the fifth takes the batch over the cap.
        let input = aliased_batch_input(MIB);
        let decode_gas = input_decode_gas(&input);

        // One gas short of the input charge: out of gas in the pre-check, so no argument is
        // decoded.
        precompiles()
            .prepare_test(Account::Alice, Account::Precompile, input.clone())
            .with_target_gas(Some(decode_gas - 1))
            .execute_error(ExitError::OutOfGas);

        // Exactly the input charge: the pre-check passes and decode reports the batch cap.
        precompiles()
            .prepare_test(Account::Alice, Account::Precompile, input)
            .with_target_gas(Some(decode_gas))
            .execute_reverts(reverts_with(&over_batch_cap(4)));
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
        let input = PCall::verify {
            chain_key: 1,
            height: 2,
            encoded_transaction: vec![0x55u8; MIB].into(),
            merkle_proof: small_merkle_proof(),
            continuity_proof: small_continuity_proof(),
        }
        .encode();
        let expected = input_decode_gas(&input)
            + KECCAK_BASE_COST
            + KECCAK_WORD_COST * words(MIB + 1)
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

/// xorshift64*: deterministic. One generator runs through every case, so a failing case
/// reproduces by re-running the test, not from its case number alone.
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
        // A well-formed encoding: up to 12 items (the batch limit is 10), mostly small, with
        // every 50th case large enough for repeated offsets to reach the batch cap.
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
        // offsets repeat, point backwards or out of bounds, and lengths are wrong; then
        // truncate.
        let words = input.len() / 32;
        for _ in 0..rng.below(4) {
            let at = 32 * rng.below(words);
            let value: u64 = match rng.below(5) {
                0 => 0,
                1 => 32 * rng.below(words) as u64,
                2 => rng.below(input.len() + 64) as u64,
                3 => rng.next() >> rng.below(64),
                // Copy another word, which is how two offsets come to point at one item.
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
