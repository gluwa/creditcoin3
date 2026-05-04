//! Transaction-pool prevalidation for `commit_attestation`.
//!
//! Attestor wallets were being drained because every attestor in the active
//! set races to submit `commit_attestation` for the same digest. The
//! transaction pool admitted every submission (the call is `Pays::Yes` up
//! front so it is not free, but the pool itself does not enforce the pallet's
//! domain checks). All race losers then paid the inclusion fee even though
//! the on-chain extrinsic returned `AttestationExists` or
//! `AttestorNotActive`.
//!
//! `Pays::No` only runs on the success post-dispatch path, so failed
//! extrinsics still pay the fee. To stop the drain we reject the obvious
//! losing cases at txpool admission time using a `SignedExtension`, before
//! any fee is charged.
//!
//! The pallet still performs the same checks inside `commit_attestation` as
//! defense in depth, in case a transaction slips past the extension (e.g.
//! state changed between admission and dispatch).

use core::{fmt::Debug, marker::PhantomData};

use frame_support::{pallet_prelude::TypeInfo, traits::IsSubType};
use parity_scale_codec::{Decode, Encode};
use sp_runtime::{
    traits::{DispatchInfoOf, SignedExtension},
    transaction_validity::{
        InvalidTransaction, TransactionValidity, TransactionValidityError, ValidTransaction,
    },
};
use sp_std::collections::btree_set::BTreeSet;

use crate::pallet::{ActiveAttestors, Call, Config, Pallet};

/// `SignedExtension` that pre-validates `commit_attestation` calls in the
/// transaction pool, rejecting:
///
/// * calls signed by accounts that are not in the active attestor set for the
///   target chain, and
/// * calls for an attestation digest that is already stored on chain (or has
///   been superseded by a later checkpoint).
///
/// Both rejections happen *before* fees are charged, eliminating the drain
/// vector where attestors lose a race and still pay the inclusion fee.
///
/// Other calls are passed through untouched.
#[derive(Encode, Decode, Clone, Eq, PartialEq, TypeInfo)]
#[scale_info(skip_type_params(T))]
pub struct PrevalidateAttestationCommit<T>(PhantomData<fn(T)>);

impl<T> Default for PrevalidateAttestationCommit<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> PrevalidateAttestationCommit<T> {
    /// Construct a new instance of the extension.
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<T: Config + Send + Sync> Debug for PrevalidateAttestationCommit<T>
where
    <T as frame_system::Config>::RuntimeCall: IsSubType<Call<T>>,
{
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        core::write!(f, "PrevalidateAttestationCommit")
    }
}

impl<T: Config + Send + Sync> SignedExtension for PrevalidateAttestationCommit<T>
where
    <T as frame_system::Config>::RuntimeCall: IsSubType<Call<T>>,
{
    type AccountId = T::AccountId;
    type Call = <T as frame_system::Config>::RuntimeCall;
    type AdditionalSigned = ();
    type Pre = ();

    const IDENTIFIER: &'static str = "PrevalidateAttestationCommit";

    fn additional_signed(&self) -> Result<Self::AdditionalSigned, TransactionValidityError> {
        Ok(())
    }

    fn pre_dispatch(
        self,
        who: &Self::AccountId,
        call: &Self::Call,
        info: &DispatchInfoOf<Self::Call>,
        len: usize,
    ) -> Result<Self::Pre, TransactionValidityError> {
        self.validate(who, call, info, len).map(|_| ())
    }

    fn validate(
        &self,
        who: &Self::AccountId,
        call: &Self::Call,
        _info: &DispatchInfoOf<Self::Call>,
        _len: usize,
    ) -> TransactionValidity {
        if let Some(Call::commit_attestation { attestation }) = call.is_sub_type() {
            let chain_key = attestation.chain_key();

            let active_attestors = ActiveAttestors::<T>::get(chain_key)
                .into_iter()
                .collect::<BTreeSet<_>>();
            if active_attestors.contains(who) && Pallet::<T>::check_duplicate(attestation) {
                return Err(TransactionValidityError::Invalid(InvalidTransaction::Stale));
            }
        }

        Ok(ValidTransaction::default())
    }
}
