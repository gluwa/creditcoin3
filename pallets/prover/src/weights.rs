#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]

use frame_support::{traits::Get, weights::Weight};
use sp_std::marker::PhantomData;

/// Weight functions for `crate`.
pub struct WeightInfo<T>(PhantomData<T>);
	/// Storage: `Prover::Provers` (r:1 w:1)
	/// Proof: `Prover::Provers` (`max_values`: None, `max_size`: None, mode: `Measured`)
	/// Storage: `Prover::CounterForProvers` (r:1 w:1)
	/// Proof: `Prover::CounterForProvers` (`max_values`: Some(1), `max_size`: Some(4), added: 499, mode: `MaxEncodedLen`)
	/// Storage: `System::Number` (r:1 w:0)
	/// Proof: `System::Number` (`max_values`: Some(1), `max_size`: Some(4), added: 499, mode: `MaxEncodedLen`)
	/// Storage: `System::ExecutionPhase` (r:1 w:0)
	/// Proof: `System::ExecutionPhase` (`max_values`: Some(1), `max_size`: Some(5), added: 500, mode: `MaxEncodedLen`)
	/// Storage: `System::EventCount` (r:1 w:1)
	/// Proof: `System::EventCount` (`max_values`: Some(1), `max_size`: Some(4), added: 499, mode: `MaxEncodedLen`)
	/// Storage: `System::Events` (r:1 w:1)
	/// Proof: `System::Events` (`max_values`: Some(1), `max_size`: None, mode: `Measured`)
impl<T: frame_system::Config> crate::pallet::WeightInfo for WeightInfo<T> {
	fn register_prover() -> Weight {
		Weight::from_parts(1, 1)
	}

    fn set_chain_price_config() -> Weight {
        Weight::from_parts(1,1)
    }

    fn unset_chain_price_config() -> Weight {
        Weight::from_parts(1,1)
    }
}
