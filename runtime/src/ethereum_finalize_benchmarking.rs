//! Measures `pallet_ethereum`'s `on_finalize`, which turns every pending
//! Ethereum transaction into the block's transactions, receipts, bloom and
//! receipts root, and computes the intermediate state root.
//!
//! Each single-component benchmark scales one input that a block's gas or
//! length budget can make large. `on_finalize_full_block` scales them all at
//! once, so the fitted weight also covers their interaction.

use alloc::vec;
use alloc::vec::Vec;
use core::marker::PhantomData;
use ethereum::{legacy::TransactionSignature, EIP658ReceiptData, LegacyTransaction, Log};
use frame_benchmarking::v2::*;
use frame_support::traits::Hooks;
use frame_system::pallet_prelude::BlockNumberFor;
use pallet_ethereum::{Pending, Receipt, Transaction, TransactionAction, TransactionStatus};
use sp_core::{H160, H256, U256};

pub struct Pallet<T: Config>(PhantomData<T>);
pub trait Config: pallet_ethereum::Config {}

const KIB: usize = 1024;

fn transaction(nonce: u32, input_len: usize) -> Transaction {
    Transaction::Legacy(LegacyTransaction {
        nonce: nonce.into(),
        gas_price: U256::one(),
        gas_limit: U256::from(21_000),
        action: TransactionAction::Call(H160::repeat_byte(0xaa)),
        value: U256::zero(),
        input: vec![0; input_len],
        signature: TransactionSignature::new(38, H256::repeat_byte(1), H256::repeat_byte(2))
            .expect("valid signature; qed"),
    })
}

/// Stores `n` pending transactions exactly as `apply_validated_transaction`
/// would. Transaction `i` carries the logs and calldata length in
/// `payloads[i]`; the rest carry neither.
fn insert_pending<T: Config>(n: u32, payloads: Vec<(Vec<Log>, usize)>) {
    let mut payloads = payloads.into_iter();
    for index in 0..n {
        let (logs, input_len) = payloads.next().unwrap_or_default();
        let transaction = transaction(index, input_len);
        let status = TransactionStatus {
            transaction_hash: transaction.hash(),
            transaction_index: index,
            from: H160::repeat_byte(0xbb),
            to: Some(H160::repeat_byte(0xaa)),
            contract_address: None,
            logs: logs.clone(),
            logs_bloom: Default::default(),
        };
        let receipt = Receipt::Legacy(EIP658ReceiptData {
            status_code: 1,
            used_gas: U256::from(21_000u64 * (index as u64 + 1)),
            logs_bloom: Default::default(),
            logs,
        });
        Pending::<T>::insert(index, (transaction, status, receipt));
    }
}

fn logs(count: u32, topics: usize, data_len: usize) -> Vec<Log> {
    (0..count)
        .map(|i| Log {
            address: H160::from_low_u64_be(i as u64 + 1),
            topics: (0..topics)
                .map(|t| H256::from_low_u64_be((i as u64) << 8 | t as u64))
                .collect(),
            data: vec![0xcd; data_len],
        })
        .collect()
}

fn storage_slot(i: u32) -> (H160, H256) {
    (
        H160::from_low_u64_be(i as u64 + 1),
        H256::from_low_u64_be(i as u64),
    )
}

/// Creates `k` EVM storage slots in the committed state.
fn create_storage<T: Config>(k: u32) {
    for i in 0..k {
        let (address, slot) = storage_slot(i);
        pallet_evm::AccountStorages::<T>::insert(address, slot, H256::repeat_byte(1));
    }
}

/// Changes the `k` slots made by `create_storage`, as `SSTORE`s earlier in the
/// block would, so the intermediate state root has `k` dirty keys to hash.
fn dirty_storage<T: Config>(k: u32) {
    for i in 0..k {
        let (address, slot) = storage_slot(i);
        pallet_evm::AccountStorages::<T>::insert(address, slot, H256::repeat_byte(2));
    }
}

fn finalize<T: Config>() {
    pallet_ethereum::Pallet::<T>::on_finalize(frame_system::Pallet::<T>::block_number());
}

fn set_block_number<T: Config>() {
    frame_system::Pallet::<T>::set_block_number(BlockNumberFor::<T>::from(2u32));
}

#[cfg(test)]
pub fn new_test_ext() -> sp_io::TestExternalities {
    use sp_runtime::BuildStorage;

    frame_system::GenesisConfig::<crate::Runtime>::default()
        .build_storage()
        .expect("frame_system genesis builds; qed")
        .into()
}

#[benchmarks]
mod benchmarks {
    use super::*;

    /// `n` transactions without logs. 75M gas / 21k gas caps a block at 3571.
    #[benchmark]
    fn on_finalize_transactions(n: Linear<0, 4_000>) {
        insert_pending::<T>(n, Vec::new());
        set_block_number::<T>();

        #[block]
        {
            finalize::<T>();
        }

        assert_eq!(Pending::<T>::count(), 0);
    }

    /// One transaction with `l` topic-less, empty logs. LOG0 costs ~381 gas,
    /// so a 75M gas block caps at ~197k.
    #[benchmark]
    fn on_finalize_logs(l: Linear<0, 200_000>) {
        insert_pending::<T>(1, vec![(logs(l, 0, 0), 0)]);
        set_block_number::<T>();

        #[block]
        {
            finalize::<T>();
        }

        assert_eq!(Pending::<T>::count(), 0);
    }

    /// One transaction with `l` empty LOG4 logs. LOG4 costs ~1890 gas, so a
    /// 75M gas block caps at ~40k logs / 160k topics.
    #[benchmark]
    fn on_finalize_topics(l: Linear<0, 40_000>) {
        insert_pending::<T>(1, vec![(logs(l, 4, 0), 0)]);
        set_block_number::<T>();

        #[block]
        {
            finalize::<T>();
        }

        assert_eq!(Pending::<T>::count(), 0);
    }

    /// One transaction with `d` LOG0 logs of 1 KiB each. A 1 KiB LOG0 costs
    /// ~8.6k gas, so a 75M gas block caps at ~8.7k KiB.
    #[benchmark]
    fn on_finalize_log_data(d: Linear<0, 9_000>) {
        insert_pending::<T>(1, vec![(logs(d, 0, KIB), 0)]);
        set_block_number::<T>();

        #[block]
        {
            finalize::<T>();
        }

        assert_eq!(Pending::<T>::count(), 0);
    }

    /// One transaction with `s` KiB of calldata, up to the 5 MiB block length.
    #[benchmark]
    fn on_finalize_transaction_size(s: Linear<0, 5_120>) {
        insert_pending::<T>(1, vec![(Vec::new(), s as usize * KIB)]);
        set_block_number::<T>();

        #[block]
        {
            finalize::<T>();
        }

        assert_eq!(Pending::<T>::count(), 0);
    }

    /// `k` EVM storage slots changed earlier in the block. Changing an existing
    /// slot costs 5000 gas, so a 75M gas block caps at 15k. The measured block
    /// includes writing the slots, so this slightly overstates the root cost.
    #[benchmark]
    fn on_finalize_state_root(k: Linear<0, 15_000>) {
        create_storage::<T>(k);
        insert_pending::<T>(1, Vec::new());
        set_block_number::<T>();

        #[block]
        {
            dirty_storage::<T>(k);
            finalize::<T>();
        }

        assert_eq!(Pending::<T>::count(), 0);
    }

    /// Every input at once. Taking all of them at their maximum together is
    /// more than one block's gas buys, so this bounds the worst case from above.
    /// The LOG4 logs, the log data and the calldata each ride in their own
    /// transaction: together in one they would encode past the runtime
    /// allocator's 32 MiB single-allocation limit, which no real block can do.
    #[benchmark]
    fn on_finalize_full_block(
        n: Linear<3, 3_600>,
        l: Linear<0, 40_000>,
        d: Linear<0, 9_000>,
        s: Linear<0, 5_120>,
        k: Linear<0, 15_000>,
    ) {
        create_storage::<T>(k);
        insert_pending::<T>(
            n,
            vec![
                (logs(l, 4, 0), 0),
                (logs(d, 0, KIB), 0),
                (Vec::new(), s as usize * KIB),
            ],
        );
        set_block_number::<T>();

        #[block]
        {
            dirty_storage::<T>(k);
            finalize::<T>();
        }

        assert_eq!(Pending::<T>::count(), 0);
    }

    impl_benchmark_test_suite!(
        Pallet,
        crate::ethereum_finalize_benchmarking::new_test_ext(),
        crate::Runtime
    );
}
