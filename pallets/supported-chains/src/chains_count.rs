//! One-off initialisation of [`SupportedChainsCount`] on chains that registered chains before it
//! existed.

use frame_support::{pallet_prelude::*, traits::OnRuntimeUpgrade, weights::Weight};
use sp_std::marker::PhantomData;

#[cfg(feature = "try-runtime")]
use sp_runtime::TryRuntimeError;
#[cfg(feature = "try-runtime")]
use sp_std::vec::Vec;

use crate::pallet::{Config, SupportedChains, SupportedChainsCount};

/// Sets [`SupportedChainsCount`] to the number of [`SupportedChains`] entries.
///
/// Deliberately not gated on the pallet's storage version, and it leaves that version alone: the
/// count is a plain new value, and other branches of this pallet use the version for unrelated
/// migrations. It rewrites the count only when it differs, so running it on every upgrade is cheap
/// and harmless.
pub struct InitSupportedChainsCount<T>(PhantomData<T>);

impl<T: Config> OnRuntimeUpgrade for InitSupportedChainsCount<T> {
    fn on_runtime_upgrade() -> Weight {
        let actual = SupportedChains::<T>::iter_keys().count() as u32;
        let reads = u64::from(actual).saturating_add(1);

        if SupportedChainsCount::<T>::get() == actual {
            return T::DbWeight::get().reads(reads);
        }

        SupportedChainsCount::<T>::put(actual);
        log::info!(
            target: "runtime::supported-chains",
            "SupportedChainsCount initialised to {actual}"
        );

        T::DbWeight::get().reads_writes(reads, 1)
    }

    #[cfg(feature = "try-runtime")]
    fn pre_upgrade() -> Result<Vec<u8>, TryRuntimeError> {
        use parity_scale_codec::Encode;

        Ok((SupportedChains::<T>::iter_keys().count() as u32).encode())
    }

    #[cfg(feature = "try-runtime")]
    fn post_upgrade(state: Vec<u8>) -> Result<(), TryRuntimeError> {
        use parity_scale_codec::Decode;

        let expected: u32 =
            Decode::decode(&mut &state[..]).map_err(|_| "failed to decode pre_upgrade state")?;

        ensure!(
            SupportedChainsCount::<T>::get() == expected,
            "SupportedChainsCount does not match the number of supported chains"
        );

        Ok(())
    }
}
