#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]

use frame_support::{traits::Get, weights::Weight};
use sp_std::marker::PhantomData;

/// Weight functions for `crate`.
pub struct WeightInfo<T>(PhantomData<T>);
impl<T: frame_system::Config> crate::WeightInfo for WeightInfo<T> {
	fn add_authority() -> Weight {
		Weight::from_parts(1, 1)
	}

	fn approve_collection() -> Weight {
		Weight::from_parts(1, 1)
	}

	fn remove_authority() -> Weight {
		Weight::from_parts(1, 1)
	}
}
