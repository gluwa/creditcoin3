
//! Benchmark coefficients for `pallet_ethereum` block assembly.
//! Adapted to inherent methods because these runtime benchmarks are outside Frontier.
//!
//! THIS FILE WAS AUTO-GENERATED USING THE SUBSTRATE BENCHMARK CLI VERSION 53.0.0
//! DATE: 2026-10-13, STEPS: `50`, REPEAT: `20`, LOW RANGE: `[]`, HIGH RANGE: `[]`
//! WORST CASE MAP SIZE: `1000000`
//! HOSTNAME: `localhost`, CPU: `AMD EPYC 7713 64-Core Processor`
//! WASM-EXECUTION: `Compiled`, CHAIN: `Some("dev")`, DB CACHE: 1024

// Executed Command:
// ./target/release/creditcoin3-node
// benchmark
// pallet
// --chain
// dev
// --steps=50
// --repeat=20
// --pallet
// pallet_ethereum
// --wasm-execution=compiled
// --heap-pages=10000
// --extrinsic=on_finalize_transactions,on_finalize_logs,on_finalize_topics,on_finalize_log_data,on_finalize_transaction_size,on_finalize_state_root
// --output
// /home/ubuntu/actions-runner/_work/_temp/pallet-ethereum-weights/single_component.rs

#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]
#![allow(missing_docs)]

use frame_support::{traits::Get, weights::Weight};
use core::marker::PhantomData;

/// Weight functions for `pallet_ethereum`.
pub struct WeightInfo<T>(PhantomData<T>);
impl<T: frame_system::Config> WeightInfo<T> {
	/// Storage: `System::Number` (r:1 w:0)
	/// Proof: `System::Number` (`max_values`: Some(1), `max_size`: Some(4), added: 499, mode: `MaxEncodedLen`)
	/// Storage: `System::Digest` (r:1 w:1)
	/// Proof: `System::Digest` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::CounterForPending` (r:1 w:1)
	/// Proof: `Ethereum::CounterForPending` (`max_values`: Some(1), `max_size`: Some(4), added: 499, mode: `MaxEncodedLen`)
	/// Storage: `Ethereum::Pending` (r:4000 w:4000)
	/// Proof: `Ethereum::Pending` (`max_values`: None, `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::BlockHash` (r:1 w:1)
	/// Proof: `Ethereum::BlockHash` (`max_values`: None, `max_size`: None, mode: `Measured`)
	/// Storage: `Timestamp::Now` (r:1 w:0)
	/// Proof: `Timestamp::Now` (`max_values`: Some(1), `max_size`: Some(8), added: 503, mode: `MaxEncodedLen`)
	/// Storage: `Ethereum::CurrentTransactionStatuses` (r:0 w:1)
	/// Proof: `Ethereum::CurrentTransactionStatuses` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::CurrentReceipts` (r:0 w:1)
	/// Proof: `Ethereum::CurrentReceipts` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::CurrentBlock` (r:0 w:1)
	/// Proof: `Ethereum::CurrentBlock` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	/// The range of component `n` is `[0, 4000]`.
	pub fn on_finalize_transactions(n: u32, ) -> Weight {
		// Proof Size summary in bytes:
		//  Measured:  `1916 + n * (858 ±0)`
		//  Estimated: `4840 + n * (3333 ±0)`
		// Minimum execution time: 63_371_000 picoseconds.
		Weight::from_parts(90_571_000, 0)
			.saturating_add(Weight::from_parts(0, 4840))
			// Standard Error: 168_363
			.saturating_add(Weight::from_parts(46_766_281, 0).saturating_mul(n.into()))
			.saturating_add(T::DbWeight::get().reads(5))
			.saturating_add(T::DbWeight::get().reads((1_u64).saturating_mul(n.into())))
			.saturating_add(T::DbWeight::get().writes(6))
			.saturating_add(T::DbWeight::get().writes((1_u64).saturating_mul(n.into())))
			.saturating_add(Weight::from_parts(0, 3333).saturating_mul(n.into()))
	}
	/// Storage: `System::Number` (r:1 w:0)
	/// Proof: `System::Number` (`max_values`: Some(1), `max_size`: Some(4), added: 499, mode: `MaxEncodedLen`)
	/// Storage: `System::Digest` (r:1 w:1)
	/// Proof: `System::Digest` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::CounterForPending` (r:1 w:1)
	/// Proof: `Ethereum::CounterForPending` (`max_values`: Some(1), `max_size`: Some(4), added: 499, mode: `MaxEncodedLen`)
	/// Storage: `Ethereum::Pending` (r:1 w:1)
	/// Proof: `Ethereum::Pending` (`max_values`: None, `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::BlockHash` (r:1 w:1)
	/// Proof: `Ethereum::BlockHash` (`max_values`: None, `max_size`: None, mode: `Measured`)
	/// Storage: `Timestamp::Now` (r:1 w:0)
	/// Proof: `Timestamp::Now` (`max_values`: Some(1), `max_size`: Some(8), added: 503, mode: `MaxEncodedLen`)
	/// Storage: `Ethereum::CurrentTransactionStatuses` (r:0 w:1)
	/// Proof: `Ethereum::CurrentTransactionStatuses` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::CurrentReceipts` (r:0 w:1)
	/// Proof: `Ethereum::CurrentReceipts` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::CurrentBlock` (r:0 w:1)
	/// Proof: `Ethereum::CurrentBlock` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	/// The range of component `l` is `[0, 200000]`.
	pub fn on_finalize_logs(l: u32, ) -> Weight {
		// Proof Size summary in bytes:
		//  Measured:  `1162 + l * (44 ±0)`
		//  Estimated: `4625 + l * (44 ±0)`
		// Minimum execution time: 119_423_000 picoseconds.
		Weight::from_parts(14_608_769_071, 0)
			.saturating_add(Weight::from_parts(0, 4625))
			// Standard Error: 18_116
			.saturating_add(Weight::from_parts(2_838_038, 0).saturating_mul(l.into()))
			.saturating_add(T::DbWeight::get().reads(6))
			.saturating_add(T::DbWeight::get().writes(7))
			.saturating_add(Weight::from_parts(0, 44).saturating_mul(l.into()))
	}
	/// Storage: `System::Number` (r:1 w:0)
	/// Proof: `System::Number` (`max_values`: Some(1), `max_size`: Some(4), added: 499, mode: `MaxEncodedLen`)
	/// Storage: `System::Digest` (r:1 w:1)
	/// Proof: `System::Digest` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::CounterForPending` (r:1 w:1)
	/// Proof: `Ethereum::CounterForPending` (`max_values`: Some(1), `max_size`: Some(4), added: 499, mode: `MaxEncodedLen`)
	/// Storage: `Ethereum::Pending` (r:1 w:1)
	/// Proof: `Ethereum::Pending` (`max_values`: None, `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::BlockHash` (r:1 w:1)
	/// Proof: `Ethereum::BlockHash` (`max_values`: None, `max_size`: None, mode: `Measured`)
	/// Storage: `Timestamp::Now` (r:1 w:0)
	/// Proof: `Timestamp::Now` (`max_values`: Some(1), `max_size`: Some(8), added: 503, mode: `MaxEncodedLen`)
	/// Storage: `Ethereum::CurrentTransactionStatuses` (r:0 w:1)
	/// Proof: `Ethereum::CurrentTransactionStatuses` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::CurrentReceipts` (r:0 w:1)
	/// Proof: `Ethereum::CurrentReceipts` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::CurrentBlock` (r:0 w:1)
	/// Proof: `Ethereum::CurrentBlock` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	/// The range of component `l` is `[0, 40000]`.
	pub fn on_finalize_topics(l: u32, ) -> Weight {
		// Proof Size summary in bytes:
		//  Measured:  `1157 + l * (300 ±0)`
		//  Estimated: `4622 + l * (300 ±0)`
		// Minimum execution time: 125_343_000 picoseconds.
		Weight::from_parts(136_333_000, 0)
			.saturating_add(Weight::from_parts(0, 4622))
			// Standard Error: 39_390
			.saturating_add(Weight::from_parts(15_691_448, 0).saturating_mul(l.into()))
			.saturating_add(T::DbWeight::get().reads(6))
			.saturating_add(T::DbWeight::get().writes(7))
			.saturating_add(Weight::from_parts(0, 300).saturating_mul(l.into()))
	}
	/// Storage: `System::Number` (r:1 w:0)
	/// Proof: `System::Number` (`max_values`: Some(1), `max_size`: Some(4), added: 499, mode: `MaxEncodedLen`)
	/// Storage: `System::Digest` (r:1 w:1)
	/// Proof: `System::Digest` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::CounterForPending` (r:1 w:1)
	/// Proof: `Ethereum::CounterForPending` (`max_values`: Some(1), `max_size`: Some(4), added: 499, mode: `MaxEncodedLen`)
	/// Storage: `Ethereum::Pending` (r:1 w:1)
	/// Proof: `Ethereum::Pending` (`max_values`: None, `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::BlockHash` (r:1 w:1)
	/// Proof: `Ethereum::BlockHash` (`max_values`: None, `max_size`: None, mode: `Measured`)
	/// Storage: `Timestamp::Now` (r:1 w:0)
	/// Proof: `Timestamp::Now` (`max_values`: Some(1), `max_size`: Some(8), added: 503, mode: `MaxEncodedLen`)
	/// Storage: `Ethereum::CurrentTransactionStatuses` (r:0 w:1)
	/// Proof: `Ethereum::CurrentTransactionStatuses` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::CurrentReceipts` (r:0 w:1)
	/// Proof: `Ethereum::CurrentReceipts` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::CurrentBlock` (r:0 w:1)
	/// Proof: `Ethereum::CurrentBlock` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	/// The range of component `d` is `[0, 9000]`.
	pub fn on_finalize_log_data(d: u32, ) -> Weight {
		// Proof Size summary in bytes:
		//  Measured:  `1158 + d * (2094 ±0)`
		//  Estimated: `4623 + d * (2094 ±0)`
		// Minimum execution time: 150_023_000 picoseconds.
		Weight::from_parts(166_133_000, 0)
			.saturating_add(Weight::from_parts(0, 4623))
			// Standard Error: 168_845
			.saturating_add(Weight::from_parts(36_102_040, 0).saturating_mul(d.into()))
			.saturating_add(T::DbWeight::get().reads(6))
			.saturating_add(T::DbWeight::get().writes(7))
			.saturating_add(Weight::from_parts(0, 2094).saturating_mul(d.into()))
	}
	/// Storage: `System::Number` (r:1 w:0)
	/// Proof: `System::Number` (`max_values`: Some(1), `max_size`: Some(4), added: 499, mode: `MaxEncodedLen`)
	/// Storage: `System::Digest` (r:1 w:1)
	/// Proof: `System::Digest` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::CounterForPending` (r:1 w:1)
	/// Proof: `Ethereum::CounterForPending` (`max_values`: Some(1), `max_size`: Some(4), added: 499, mode: `MaxEncodedLen`)
	/// Storage: `Ethereum::Pending` (r:1 w:1)
	/// Proof: `Ethereum::Pending` (`max_values`: None, `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::BlockHash` (r:1 w:1)
	/// Proof: `Ethereum::BlockHash` (`max_values`: None, `max_size`: None, mode: `Measured`)
	/// Storage: `Timestamp::Now` (r:1 w:0)
	/// Proof: `Timestamp::Now` (`max_values`: Some(1), `max_size`: Some(8), added: 503, mode: `MaxEncodedLen`)
	/// Storage: `Ethereum::CurrentTransactionStatuses` (r:0 w:1)
	/// Proof: `Ethereum::CurrentTransactionStatuses` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::CurrentReceipts` (r:0 w:1)
	/// Proof: `Ethereum::CurrentReceipts` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::CurrentBlock` (r:0 w:1)
	/// Proof: `Ethereum::CurrentBlock` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	/// The range of component `s` is `[0, 5120]`.
	pub fn on_finalize_transaction_size(s: u32, ) -> Weight {
		// Proof Size summary in bytes:
		//  Measured:  `1159 + s * (1024 ±0)`
		//  Estimated: `4624 + s * (1024 ±0)`
		// Minimum execution time: 118_832_000 picoseconds.
		Weight::from_parts(676_047_702, 0)
			.saturating_add(Weight::from_parts(0, 4624))
			// Standard Error: 164_559
			.saturating_add(Weight::from_parts(35_029_711, 0).saturating_mul(s.into()))
			.saturating_add(T::DbWeight::get().reads(6))
			.saturating_add(T::DbWeight::get().writes(7))
			.saturating_add(Weight::from_parts(0, 1024).saturating_mul(s.into()))
	}
	/// Storage: `System::Number` (r:1 w:0)
	/// Proof: `System::Number` (`max_values`: Some(1), `max_size`: Some(4), added: 499, mode: `MaxEncodedLen`)
	/// Storage: `System::Digest` (r:1 w:1)
	/// Proof: `System::Digest` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::CounterForPending` (r:1 w:1)
	/// Proof: `Ethereum::CounterForPending` (`max_values`: Some(1), `max_size`: Some(4), added: 499, mode: `MaxEncodedLen`)
	/// Storage: `Ethereum::Pending` (r:1 w:1)
	/// Proof: `Ethereum::Pending` (`max_values`: None, `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::BlockHash` (r:1 w:1)
	/// Proof: `Ethereum::BlockHash` (`max_values`: None, `max_size`: None, mode: `Measured`)
	/// Storage: `Timestamp::Now` (r:1 w:0)
	/// Proof: `Timestamp::Now` (`max_values`: Some(1), `max_size`: Some(8), added: 503, mode: `MaxEncodedLen`)
	/// Storage: `EVM::AccountStorages` (r:0 w:14999)
	/// Proof: `EVM::AccountStorages` (`max_values`: None, `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::CurrentTransactionStatuses` (r:0 w:1)
	/// Proof: `Ethereum::CurrentTransactionStatuses` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::CurrentReceipts` (r:0 w:1)
	/// Proof: `Ethereum::CurrentReceipts` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	/// Storage: `Ethereum::CurrentBlock` (r:0 w:1)
	/// Proof: `Ethereum::CurrentBlock` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
	/// The range of component `k` is `[0, 15000]`.
	pub fn on_finalize_state_root(k: u32, ) -> Weight {
		// Proof Size summary in bytes:
		//  Measured:  `2741 + k * (121 ±0)`
		//  Estimated: `6075 + k * (122 ±0)`
		// Minimum execution time: 113_842_000 picoseconds.
		Weight::from_parts(126_803_000, 0)
			.saturating_add(Weight::from_parts(0, 6075))
			// Standard Error: 65_488
			.saturating_add(Weight::from_parts(19_259_746, 0).saturating_mul(k.into()))
			.saturating_add(T::DbWeight::get().reads(6))
			.saturating_add(T::DbWeight::get().writes(7))
			.saturating_add(T::DbWeight::get().writes((1_u64).saturating_mul(k.into())))
			.saturating_add(Weight::from_parts(0, 122).saturating_mul(k.into()))
	}
}
