#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]

use frame_support::{traits::Get, weights::Weight};
use sp_std::marker::PhantomData;

/// Weight functions for `crate`.
pub struct WeightInfo<T>(PhantomData<T>);
impl<T: frame_system::Config> crate::WeightInfo for WeightInfo<T> {
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
