#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]

use frame_support::{traits::Get, weights::Weight};
use sp_std::marker::PhantomData;

/// Weight functions for `crate`.
pub struct WeightInfo<T>(PhantomData<T>);
impl<T: frame_system::Config> crate::pallet::WeightInfo for WeightInfo<T> {

	fn register_attestor() -> Weight {
		Weight::from_parts(1, 1)
	}

	fn unregister_attestor() -> Weight {
		Weight::from_parts(1,1)
	}

	fn set_max_attestors() -> Weight {
		Weight::from_parts(1,1)
	}

	fn register_invulnerable() -> Weight {
		Weight::from_parts(1,1)
	}

	fn unregister_invulnerable() -> Weight {
		Weight::from_parts(1,1)
	}

	fn set_max_invulnerables() -> Weight {
		Weight::from_parts(1,1)
	}

	fn attest_block() -> Weight {
		Weight::from_parts(1,1)
	}

	fn bootstrap_chain() -> Weight {
		Weight::from_parts(1,1)
	}

	fn commit_attestation() -> Weight {
		Weight::from_parts(1,1)
	}

	fn set_comitte_set_size() -> Weight {
		Weight::from_parts(1,1)
	}

	fn add_supported_chain() -> Weight {
		Weight::from_parts(1,1)
	}

	fn remove_supported_chain() -> Weight {
		Weight::from_parts(1,1)
	}
}
