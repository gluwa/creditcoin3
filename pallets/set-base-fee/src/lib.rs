#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {
    use frame_support::{pallet_prelude::*, traits::EnsureOrigin};
    use frame_system::pallet_prelude::*;
    use sp_core::U256;

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    #[pallet::config]
    pub trait Config: frame_system::Config {
        /// The overarching event type.
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        /// The origin which may set the base fee.
        type UpdateOrigin: EnsureOrigin<Self::RuntimeOrigin>;

        /// The default value to use when no value is set.
        type DefaultBaseFee: Get<U256>;
    }

    #[pallet::type_value]
    pub fn DefaultBaseFee<T: Config>() -> U256 {
        T::DefaultBaseFee::get()
    }

    #[pallet::storage]
    #[pallet::getter(fn base_fee_per_gas)]
    pub type BaseFeePerGas<T> = StorageValue<_, U256, ValueQuery, DefaultBaseFee<T>>;

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// Base fee per gas updated. [new_base_fee]
        BaseFeePerGasUpdated { fee: U256 },
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::call_index(0)]
        #[pallet::weight(T::DbWeight::get().writes(1))]
        pub fn set_base_fee_per_gas(origin: OriginFor<T>, fee: U256) -> DispatchResult {
            T::UpdateOrigin::ensure_origin(origin)?;

            BaseFeePerGas::<T>::set(fee);

            Self::deposit_event(Event::BaseFeePerGasUpdated { fee });

            Ok(())
        }
    }

    impl<T: Config> Get<U256> for Pallet<T> {
        fn get() -> U256 {
            match BaseFeePerGas::<T>::try_get() {
                Ok(value) => value,
                Err(_) => T::DefaultBaseFee::get(),
            }
        }
    }
}
