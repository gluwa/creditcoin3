//! Transaction-pool prevalidation for `commit_attestation`.
//!
//! Attestor wallets were being drained because every attestor in the active set races to submit
//!`commit_attestation` for the same digest. The transaction pool admitted every submission (the
//! call is `Pays::Yes` up front so it is not free, but the pool itself does not enforce the
//! pallet's domain checks). All race losers then paid the inclusion fee even though the on-chain
//! extrinsic returned `AttestationExists` or `AttestorNotActive`.
//!
//! `Pays::No` only runs on the success post-dispatch path, so failed extrinsics still pay the fee.
//! To stop the drain we reject the obvious losing cases at txpool admission time using a
//! `TransactionExtension`, before any fee is charged.

use core::{fmt::Debug, marker::PhantomData};

use frame_support::{
    pallet_prelude::TypeInfo,
    traits::{IsSubType, OriginTrait},
};
use parity_scale_codec::{Decode, DecodeWithMemTracking, Encode};
use sp_runtime::{
    impl_tx_ext_default,
    traits::{DispatchInfoOf, TransactionExtension, ValidateResult},
    transaction_validity::{
        InvalidTransaction, TransactionSource, TransactionValidityError, ValidTransaction,
    },
};
use sp_std::collections::btree_set::BTreeSet;

use crate::pallet::{ActiveAttestors, Call, Config, Pallet};

/// `TransactionExtension` that pre-validates `commit_attestation` calls in the transaction pool,
/// rejecting:
///
/// * calls signed by accounts that are not in the active attestor set for the target chain, and
/// * calls for an attestation digest that is already stored on chain (or has been superseded by a
///   later checkpoint).
///
/// Both rejections happen *before* fees are charged, eliminating the drain vector where attestors
/// lose a race and still pay the inclusion fee.
///
/// Other calls are passed through untouched. The extension carries no implicit payload and no
/// state between `validate` and dispatch, so `Implicit`, `Val` and `Pre` are all `()` and all the
/// logic lives in `validate`.
#[derive(Encode, Decode, DecodeWithMemTracking, Clone, Eq, PartialEq, TypeInfo)]
#[scale_info(skip_type_params(T))]
pub struct PrevalidateAttestationCommit<T>(PhantomData<fn(T)>);

const EXTENSION_IDENTIFIER: &str = "PrevalidateAttestationCommit";

impl<T> Default for PrevalidateAttestationCommit<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> PrevalidateAttestationCommit<T> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<T: Config + Send + Sync> Debug for PrevalidateAttestationCommit<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        core::write!(f, "{EXTENSION_IDENTIFIER}")
    }
}

impl<T: Config + Send + Sync> TransactionExtension<<T as frame_system::Config>::RuntimeCall>
    for PrevalidateAttestationCommit<T>
where
    <T as frame_system::Config>::RuntimeCall: IsSubType<Call<T>>,
{
    const IDENTIFIER: &'static str = EXTENSION_IDENTIFIER;
    type Implicit = ();
    type Val = ();
    type Pre = ();

    fn validate(
        &self,
        origin: <T as frame_system::Config>::RuntimeOrigin,
        call: &<T as frame_system::Config>::RuntimeCall,
        _info: &DispatchInfoOf<<T as frame_system::Config>::RuntimeCall>,
        _len: usize,
        _self_implicit: Self::Implicit,
        _inherited_implication: &impl Encode,
        _source: TransactionSource,
    ) -> ValidateResult<Self::Val, <T as frame_system::Config>::RuntimeCall> {
        // Only signed origins carry an attestor account; everything else passes through.
        if let Some(who) = origin.as_signer() {
            if let Some(Call::commit_attestation { attestation }) = call.is_sub_type() {
                let chain_key = attestation.chain_key();

                let active_attestors = ActiveAttestors::<T>::get(chain_key)
                    .into_iter()
                    .collect::<BTreeSet<_>>();
                if active_attestors.contains(who) && Pallet::<T>::check_duplicate(attestation) {
                    return Err(TransactionValidityError::Invalid(InvalidTransaction::Stale));
                }
            }
        }

        Ok((ValidTransaction::default(), (), origin))
    }

    // No implicit weight cost beyond a couple of reads that also happen at dispatch, and nothing to
    // prepare: `weight` defaults to `Weight::zero()` and `prepare` to `Ok(())`.
    impl_tx_ext_default!(<T as frame_system::Config>::RuntimeCall; weight prepare);
}
