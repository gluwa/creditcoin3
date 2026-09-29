//! Measures `pallet_ethereum`'s `on_finalize`, which turns every pending
//! Ethereum transaction into the block's transactions, receipts, bloom and
//! receipts root.
//!
//! Each benchmark scales one input that a block's gas budget can make large:
//! transaction count, log count, topic count and log data size.

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

fn transaction(nonce: u32) -> Transaction {
    Transaction::Legacy(LegacyTransaction {
        nonce: nonce.into(),
        gas_price: U256::one(),
        gas_limit: U256::from(21_000),
        action: TransactionAction::Call(H160::repeat_byte(0xaa)),
        value: U256::zero(),
        input: Vec::new(),
        signature: TransactionSignature::new(38, H256::repeat_byte(1), H256::repeat_byte(2))
            .expect("valid signature; qed"),
    })
}

/// Stores `n` pending transactions, each carrying a copy of `logs`, exactly as
/// `apply_validated_transaction` would.
fn insert_pending<T: Config>(n: u32, logs: Vec<Log>) {
    for index in 0..n {
        let transaction = transaction(index);
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
            logs: logs.clone(),
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

fn finalize<T: Config>() {
    pallet_ethereum::Pallet::<T>::on_finalize(frame_system::Pallet::<T>::block_number());
}

#[benchmarks]
mod benchmarks {
    use super::*;

    /// `n` transactions without logs. 75M gas / 21k gas caps a block at 3571.
    #[benchmark]
    fn on_finalize_transactions(n: Linear<0, 4_000>) {
        insert_pending::<T>(n, Vec::new());
        frame_system::Pallet::<T>::set_block_number(BlockNumberFor::<T>::from(2u32));

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
        insert_pending::<T>(1, logs(l, 0, 0));
        frame_system::Pallet::<T>::set_block_number(BlockNumberFor::<T>::from(2u32));

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
        insert_pending::<T>(1, logs(l, 4, 0));
        frame_system::Pallet::<T>::set_block_number(BlockNumberFor::<T>::from(2u32));

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
        insert_pending::<T>(1, logs(d, 0, 1024));
        frame_system::Pallet::<T>::set_block_number(BlockNumberFor::<T>::from(2u32));

        #[block]
        {
            finalize::<T>();
        }

        assert_eq!(Pending::<T>::count(), 0);
    }
}
