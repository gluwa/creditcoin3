#![cfg_attr(not(feature = "std"), no_std)]

use frame_support::pallet;
use pallet_bridge_grandpa::BridgedBlockHash;
use parity_scale_codec::{Decode, Encode};

pub use pallet::*;
use sp_core::ConstU32;
use sp_runtime::{BoundedVec, RuntimeDebug};
use sp_std::prelude::*;

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, scale_info::TypeInfo)]
pub struct BurnId(pub u64);

pub type MaxExternalAddressLen = ConstU32<256>;

pub type ExternalAddress = BoundedVec<u8, MaxExternalAddressLen>;

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, scale_info::TypeInfo)]
pub struct BurnInfo<AccountId, Balance> {
    pub account: AccountId,
    pub balance: Balance,
    pub collector: ExternalAddress,
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, scale_info::TypeInfo)]
pub struct BurnProof<AccountId, Balance, BridgedBlockHash, BridgedHash> {
    pub proof: Vec<Vec<u8>>,
    pub id: BurnId,
    pub info: BurnInfo<AccountId, Balance>,
    pub at: BridgedBlockHash,
    pub storage_root: BridgedHash,
}

pub type BridgedHashOf<T> =
    <<T as pallet_bridge_grandpa::Config>::BridgedChain as bp_runtime::Chain>::Hash;

pub type BurnProofOf<T> =
    BurnProof<AccountIdOf<T>, <T as Config>::Balance, BridgedBlockHash<T, ()>, BridgedHashOf<T>>;

pub type AccountIdOf<T> = <T as Config>::AccountId;

pub type BurnInfoOf<T> = BurnInfo<AccountIdOf<T>, <T as Config>::Balance>;

#[pallet(dev_mode)]
pub mod pallet {
    use crate::{AccountIdOf, BurnProofOf, ExternalAddress};
    use frame_support::pallet_prelude::*;
    use frame_support::traits::tokens::Balance;
    use frame_support::traits::Currency;
    use frame_system::pallet_prelude::*;
    use pallet_bridge_grandpa::BridgedChain;
    use sp_io::hashing::twox_128;

    #[pallet::config]
    pub trait Config: frame_system::Config + pallet_bridge_grandpa::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        type Balance: Balance;
        type AccountId: Parameter + From<[u8; 32]>;
        type Currency: Currency<<Self as Config>::AccountId, Balance = <Self as Config>::Balance>;
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        Minted(AccountIdOf<T>, T::Balance),
    }

    #[pallet::error]
    pub enum Error<T> {
        CouldntMakeChecker,
        BurnInfoNotFound,
        InvalidExternalAddress,
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::weight(0)]
        #[pallet::call_index(0)]
        pub fn mint(origin: OriginFor<T>, proof: BurnProofOf<T>) -> DispatchResultWithPostInfo {
            let _who = ensure_signed(origin)?;

            let mut checker =
                match <pallet_bridge_grandpa::Pallet<T> as bp_header_chain::HeaderChain<
                    BridgedChain<T, ()>,
                >>::storage_proof_checker(proof.at.clone(), proof.proof)
                {
                    Ok(checker) => checker,
                    Err(e) => {
                        log::error!(target: "bridgey", "Could not make checker: {:?}", e);
                        return Err(Error::<T>::CouldntMakeChecker.into());
                    }
                };

            let mut key = twox_128(b"Creditcoin").to_vec();
            key.extend_from_slice(&twox_128(b"BurnedFunds"));

            proof.id.encode_to(&mut key);

            let burn_info = match checker
                .read_and_decode_mandatory_value::<crate::BurnInfoOf<T>>(&key)
            {
                Ok(v) => v,
                Err(e) => {
                    log::error!(target: "bridgey", "Could not read and decode mandatory value: {:?}", e);
                    return Err(Error::<T>::BurnInfoNotFound.into());
                }
            };

            let collector = Self::account_id_from_external(&burn_info.collector)?;

            T::Currency::deposit_creating(&collector, burn_info.balance);
            Self::deposit_event(Event::Minted(collector, burn_info.balance));

            Ok(().into())
        }
    }

    impl<T: Config> Pallet<T> {
        fn account_id_from_external(address: &ExternalAddress) -> Result<AccountIdOf<T>, Error<T>> {
            if address.len() != 32 {
                return Err(Error::<T>::InvalidExternalAddress);
            }

            let mut account_id = [0u8; 32];
            account_id.copy_from_slice(&address[..]);

            Ok(AccountIdOf::<T>::from(account_id))
        }
    }
}
