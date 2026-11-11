//! Measures whole EVM blocks: executing their Ethereum transactions and then
//! `pallet_ethereum`'s `on_finalize`, which builds the block's receipts, bloom
//! and roots.
//!
//! Every Ethereum transaction is charged `gas_to_weight(used_gas)` and nothing
//! else, so the finalize work has to be paid for out of that same per-gas
//! weight. Each benchmark fills a block with one shape of transaction that buys
//! a lot of finalize work per unit of gas, so its fitted weight divided by the
//! gas the shape uses (see `shape_gas` in the tests) is that shape's real
//! ref_time per gas, which `tests::per_gas_charge_covers_execution_and_finalize`
//! checks against `WeightPerGas` using the committed `weights::evm_block`.

use alloc::vec;
use alloc::vec::Vec;
#[cfg(feature = "runtime-benchmarks")]
use core::marker::PhantomData;
use ethereum::{legacy::TransactionSignature, LegacyTransaction, ReceiptV4 as Receipt};
use frame_support::traits::Hooks;
use frame_system::pallet_prelude::BlockNumberFor;
use pallet_ethereum::{Transaction, TransactionAction};
use pallet_evm::FeeCalculator;
use sp_core::{H160, H256, U256};

#[cfg(feature = "runtime-benchmarks")]
pub struct Pallet<T: Config>(PhantomData<T>);

pub trait Config: pallet_ethereum::Config {
    /// Gives `who` enough balance to pay for any transaction a block can hold.
    fn fund(who: H160);
    /// Applies `transaction` from `from` as a block author would, after the
    /// signature has been checked.
    fn transact(from: H160, transaction: Transaction);
}

impl Config for crate::Runtime {
    fn fund(who: H160) {
        use frame_support::traits::Currency;
        use pallet_evm::AddressMapping;
        let account = crate::AddressMapping::into_account_id(who);
        crate::Balances::make_free_balance_be(&account, 1_000_000_000_000_000_000_000_000);
    }

    fn transact(from: H160, transaction: Transaction) {
        crate::Ethereum::transact(
            pallet_ethereum::RawOrigin::EthereumTransaction(from).into(),
            transaction,
        )
        .expect("benchmarked transactions apply; qed");
    }
}

const KIB: usize = 1024;
use crate::BLOCK_GAS_LIMIT;
const CONTRACT: H160 = H160::repeat_byte(0xcc);
const SENDER: H160 = H160::repeat_byte(0xaa);

/// How many times the loop body is repeated per iteration in `looped` code, for
/// each looped shape. Unrolling keeps the loop's own ~40 gas per iteration small
/// next to the body, as an attacker writing the contract would.
const LOG0_UNROLL: usize = 64;
const LOG4_UNROLL: usize = 16;

/// `PUSH0 PUSH0 LOG0`: an empty, topic-less log for 379 gas.
const LOG0: &[u8] = &[0x5f, 0x5f, 0xa0];
/// Six `PUSH0`s and `LOG4`: an empty log with four topics for 1887 gas.
const LOG4: &[u8] = &[0x5f, 0x5f, 0x5f, 0x5f, 0x5f, 0x5f, 0xa4];
/// `GAS GAS SSTORE`: writes a new slot. The remaining gas differs on every
/// iteration, so every key is fresh, and it is never zero.
const SSTORE_NEW: &[u8] = &[0x5a, 0x5a, 0x55];
/// `DUP1 DUP1 SSTORE`: writes the loop counter to the slot it names. With slots
/// `1..=k` already set to something else, this changes an existing slot.
const SSTORE_CHANGE: &[u8] = &[0x80, 0x80, 0x55];

/// Code that runs `body` `unroll` times per iteration, for as many iterations as
/// the first calldata word says. The counter stays on top of the stack, so a
/// body may read it with `DUP1`, but must leave the stack as it found it.
fn looped(body: &[u8], unroll: usize) -> Vec<u8> {
    // PUSH0 CALLDATALOAD; loop: JUMPDEST DUP1 ISZERO PUSH2 <end> JUMPI
    let mut code = vec![0x5f, 0x35, 0x5b, 0x80, 0x15, 0x61, 0, 0, 0x57];
    for _ in 0..unroll {
        code.extend_from_slice(body);
    }
    // PUSH1 1 SWAP1 SUB PUSH1 <loop> JUMP; end: JUMPDEST STOP
    code.extend_from_slice(&[0x60, 0x01, 0x90, 0x03, 0x60, 0x02, 0x56]);
    let end = code.len();
    code.extend_from_slice(&[0x5b, 0x00]);
    code[6..8].copy_from_slice(&(end as u16).to_be_bytes());
    code
}

fn transaction<T: Config>(
    nonce: u32,
    gas_limit: u64,
    to: H160,
    value: U256,
    input: Vec<u8>,
) -> Transaction {
    Transaction::Legacy(LegacyTransaction {
        nonce: nonce.into(),
        gas_price: <T as pallet_evm::Config>::FeeCalculator::min_gas_price().0,
        gas_limit: gas_limit.into(),
        action: TransactionAction::Call(to),
        value,
        input,
        signature: TransactionSignature::new(38, H256::repeat_byte(1), H256::repeat_byte(2))
            .expect("valid signature; qed"),
    })
}

fn deploy<T: Config>(code: Vec<u8>) {
    pallet_evm::Pallet::<T>::create_account(CONTRACT, code, None)
        .expect("installing contract code cannot fail; qed");
}

/// One transaction that runs the looped contract for `iterations` iterations.
fn call_looped<T: Config>(iterations: u32) {
    let input = U256::from(iterations).to_big_endian().to_vec();
    T::transact(
        SENDER,
        transaction::<T>(0, BLOCK_GAS_LIMIT, CONTRACT, U256::zero(), input),
    );
}

fn prepare<T: Config>() {
    frame_system::Pallet::<T>::set_block_number(BlockNumberFor::<T>::from(2u32));
    T::fund(SENDER);
}

fn finalize<T: Config>() {
    pallet_ethereum::Pallet::<T>::on_finalize(frame_system::Pallet::<T>::block_number());
}

/// Every transaction in the finalized block succeeded and the block stayed
/// within its gas limit. Returns the gas it used.
pub fn block_gas<T: Config>() -> u64 {
    let receipts = pallet_ethereum::CurrentReceipts::<T>::get().unwrap_or_default();
    assert!(!receipts.is_empty(), "the block holds no transactions");
    let mut used = U256::zero();
    for receipt in &receipts {
        let Receipt::Legacy(data) = receipt else {
            panic!("only legacy transactions are benchmarked")
        };
        assert_eq!(data.status_code, 1, "a benchmarked transaction failed");
        used = data.used_gas;
    }
    let used = used.as_u64();
    assert!(
        used <= BLOCK_GAS_LIMIT,
        "{used} gas is over the block gas limit"
    );
    used
}

/// Sets `k` storage slots of `CONTRACT`, keyed `1..=k`, to a value
/// `SSTORE_CHANGE` never writes.
fn create_storage<T: Config>(k: u32) {
    for i in 1..=k {
        pallet_evm::AccountStorages::<T>::insert(
            CONTRACT,
            H256::from_low_u64_be(i as u64),
            H256::repeat_byte(0xff),
        );
    }
}

/// Sets up and runs a block of the named shape at size `x`, without
/// finalizing it. Shared with the tests, which measure each shape's gas.
pub mod shapes {
    use super::*;

    /// The largest size of each shape, shared by its benchmark range and the
    /// per-gas test so the two cannot drift apart.
    pub const TRANSFERS_MAX: u32 = 3_571;
    pub const LOG0_MAX: u32 = 3_000;
    pub const LOG4_MAX: u32 = 2_400;
    pub const SSTORE_NEW_MAX: u32 = 3_300;
    pub const SSTORE_CHANGE_MAX: u32 = 14_400;
    pub const CALLDATA_MAX_KIB: u32 = 5_000;

    /// `n` plain value transfers, each from its own sender to a new account.
    /// 75M gas / 21k gas caps a block at 3571.
    pub fn transfers_setup<T: Config>(n: u32) -> Vec<H160> {
        prepare::<T>();
        (0..n)
            .map(|i| {
                let sender = H160::from_low_u64_be(0x1_0000_0000 + i as u64);
                T::fund(sender);
                sender
            })
            .collect()
    }

    pub fn transfers_run<T: Config>(senders: Vec<H160>) {
        for (i, sender) in senders.into_iter().enumerate() {
            let to = H160::from_low_u64_be(0x2_0000_0000 + i as u64);
            T::transact(
                sender,
                transaction::<T>(0, 21_000, to, U256::one(), Vec::new()),
            );
        }
    }

    pub fn log0_setup<T: Config>() {
        prepare::<T>();
        deploy::<T>(looped(LOG0, LOG0_UNROLL));
    }

    pub fn log4_setup<T: Config>() {
        prepare::<T>();
        deploy::<T>(looped(LOG4, LOG4_UNROLL));
    }

    pub fn sstore_new_setup<T: Config>() {
        prepare::<T>();
        deploy::<T>(looped(SSTORE_NEW, 1));
    }

    pub fn sstore_change_setup<T: Config>(k: u32) {
        prepare::<T>();
        deploy::<T>(looped(SSTORE_CHANGE, 1));
        create_storage::<T>(k);
    }

    pub fn run_looped<T: Config>(iterations: u32) {
        call_looped::<T>(iterations);
    }

    pub fn calldata_setup<T: Config>() {
        prepare::<T>();
    }

    /// One transfer carrying `s` KiB of zero calldata, the cheapest bytes to
    /// buy under the EIP-7623 floor of 10 gas each.
    pub fn calldata_run<T: Config>(s: u32) {
        T::transact(
            SENDER,
            transaction::<T>(
                0,
                BLOCK_GAS_LIMIT,
                H160::repeat_byte(0xdd),
                U256::zero(),
                vec![0; s as usize * KIB],
            ),
        );
    }
}

#[cfg(test)]
pub fn new_test_ext() -> sp_io::TestExternalities {
    use sp_runtime::BuildStorage;

    frame_system::GenesisConfig::<crate::Runtime>::default()
        .build_storage()
        .expect("frame_system genesis builds; qed")
        .into()
}

#[cfg(feature = "runtime-benchmarks")]
#[frame_benchmarking::v2::benchmarks]
mod benchmarks {
    use super::shapes::*;
    use super::*;
    use frame_benchmarking::v2::*;

    /// `n` value transfers. 75M gas / 21k caps a block at 3571.
    #[benchmark]
    fn evm_block_transfers(n: Linear<1, TRANSFERS_MAX>) {
        let senders = transfers_setup::<T>(n);

        #[block]
        {
            transfers_run::<T>(senders);
            finalize::<T>();
        }

        block_gas::<T>();
    }

    /// `i` iterations of 64 LOG0s, ~24.3k gas each, so 3000 is ~73M gas.
    #[benchmark]
    fn evm_block_log0(i: Linear<1, LOG0_MAX>) {
        log0_setup::<T>();

        #[block]
        {
            run_looped::<T>(i);
            finalize::<T>();
        }

        block_gas::<T>();
    }

    /// `i` iterations of 16 LOG4s, ~30.2k gas each, so 2400 is ~73M gas.
    #[benchmark]
    fn evm_block_log4(i: Linear<1, LOG4_MAX>) {
        log4_setup::<T>();

        #[block]
        {
            run_looped::<T>(i);
            finalize::<T>();
        }

        block_gas::<T>();
    }

    /// `i` new storage slots, ~22.1k gas each, so 3300 is ~73M gas.
    #[benchmark]
    fn evm_block_sstore_new(i: Linear<1, SSTORE_NEW_MAX>) {
        sstore_new_setup::<T>();

        #[block]
        {
            run_looped::<T>(i);
            finalize::<T>();
        }

        block_gas::<T>();
    }

    /// `i` changed existing storage slots, ~5k gas each, so 14400 is ~73M gas.
    /// Every changed slot is a dirty key for the intermediate state root.
    #[benchmark]
    fn evm_block_sstore_change(i: Linear<1, SSTORE_CHANGE_MAX>) {
        sstore_change_setup::<T>(i);

        #[block]
        {
            run_looped::<T>(i);
            finalize::<T>();
        }

        block_gas::<T>();
    }

    /// `s` KiB of zero calldata in one transaction. The 5 MiB block length caps
    /// it below 5120 KiB, well before the gas limit does.
    #[benchmark]
    fn evm_block_calldata(s: Linear<1, CALLDATA_MAX_KIB>) {
        calldata_setup::<T>();

        #[block]
        {
            calldata_run::<T>(s);
            finalize::<T>();
        }

        block_gas::<T>();
    }

    impl_benchmark_test_suite!(
        Pallet,
        crate::evm_block_benchmarking::new_test_ext(),
        crate::Runtime
    );
}

/// Only production pricing is checked. Devnet and fast-runtime fit 75M gas into
/// a third of the weight, which underprices EVM execution itself; with nothing
/// of value at stake there, that is accepted, and these tests are not built.
#[cfg(all(test, not(any(feature = "devnet", feature = "fast-runtime"))))]
mod tests {
    use super::shapes::*;
    use super::*;
    use crate::Runtime;
    use frame_support::weights::Weight;

    /// Gas each shape uses at size `x`, in a fresh block.
    fn shape_gas(shape: &str, x: u32) -> u64 {
        new_test_ext().execute_with(|| {
            match shape {
                "transfers" => transfers_run::<Runtime>(transfers_setup::<Runtime>(x)),
                "log0" => {
                    log0_setup::<Runtime>();
                    run_looped::<Runtime>(x)
                }
                "log4" => {
                    log4_setup::<Runtime>();
                    run_looped::<Runtime>(x)
                }
                "sstore_new" => {
                    sstore_new_setup::<Runtime>();
                    run_looped::<Runtime>(x)
                }
                "sstore_change" => {
                    sstore_change_setup::<Runtime>(x);
                    run_looped::<Runtime>(x)
                }
                "calldata" => {
                    calldata_setup::<Runtime>();
                    calldata_run::<Runtime>(x)
                }
                _ => unreachable!(),
            }
            finalize::<Runtime>();
            block_gas::<Runtime>()
        })
    }

    /// Every benchmarked block shape costs less ref_time, executing and
    /// finalizing, than the weight its gas is charged. `on_initialize` reserves
    /// only the fixed part of `on_finalize`, so this is what keeps the rest of
    /// it inside the block's normal budget.
    #[test]
    fn per_gas_charge_covers_execution_and_finalize() {
        use crate::weights::evm_block::WeightInfo as Bench;
        type W = Bench<Runtime>;
        /// A block shape, its largest size and its benchmarked weight function.
        type Shape = (&'static str, u32, fn(u32) -> Weight);

        let per_gas = crate::WeightPerGas::get().ref_time();

        let shapes: [Shape; 6] = [
            ("transfers", TRANSFERS_MAX, W::evm_block_transfers),
            ("log0", LOG0_MAX, W::evm_block_log0),
            ("log4", LOG4_MAX, W::evm_block_log4),
            ("sstore_new", SSTORE_NEW_MAX, W::evm_block_sstore_new),
            (
                "sstore_change",
                SSTORE_CHANGE_MAX,
                W::evm_block_sstore_change,
            ),
            ("calldata", CALLDATA_MAX_KIB, W::evm_block_calldata),
        ];
        for (shape, max, weight) in shapes {
            let (gas_low, gas_high) = (shape_gas(shape, 1), shape_gas(shape, max));
            let (low, high) = (weight(1).ref_time(), weight(max).ref_time());

            let marginal = (high - low) / (gas_high - gas_low);
            let full = high / gas_high;
            println!(
                "{shape}: {marginal} marginal, {full} full block, {per_gas} charged ref_time/gas"
            );
            assert!(
                marginal <= per_gas && full <= per_gas,
                "{shape} costs more ref_time per gas than WeightPerGas charges"
            );
        }
    }
}
