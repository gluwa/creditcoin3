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

use crate::pallet::{ActiveAttestors, Call, Config, MaxCatchup, Pallet};

/// `TransactionExtension` that pre-validates `commit_attestation` calls in the transaction pool,
/// rejecting:
///
/// * calls with over-long attestor lists or oversized continuity proofs (invalid regardless of
///   signer), and
/// * calls from *active* attestors for an attestation digest that is already stored on chain (or
///   has been superseded by a later checkpoint).
///
/// These rejections happen *before* fees are charged, eliminating the drain vector where
/// attestors lose a race and still pay the inclusion fee.
///
/// Calls signed by accounts that are **not** in the active attestor set are deliberately let
/// through: they fail in dispatch (`AttestorNotActive`) and pay fees, so a deregistered or
/// misconfigured node cannot spam free prevalidation-rejected submissions.
///
/// Admitted `commit_attestation` transactions carry a `provides` tag keyed by
/// `(chain_key, digest)` so the pool holds at most one pending submission per attestation —
/// racing duplicates from other attestors are deduplicated at admission instead of all being
/// broadcast and racing to dispatch.
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

            // Reject over-long attestor lists at admission time (mirrors the on-chain
            // `validate_attestation` `TooManyAttestors` check) so a malformed/oversized payload
            // never pays the inclusion fee. This check is signer-independent: an over-long list
            // is invalid regardless of who submitted it.
            if !Pallet::<T>::attestors_within_bound(chain_key, attestation) {
                return Err(TransactionValidityError::Invalid(
                    InvalidTransaction::ExhaustsResources,
                ));
            }

            // Reject oversized continuity proofs at txpool admission so a malicious or buggy
            // active attestor cannot force the runtime to run an unbounded keccak chain (over
            // attacker-chosen `roots: Vec<H256>`) inside dispatch. `MaxCatchup` is a *block*
            // bound (see its storage docs): each continuity proof spans at most that many
            // blocks. `max(attestation_interval)` keeps steady-state attestations (whose
            // proofs span `attestation_interval - 1` roots) admissible if `MaxCatchup` is ever
            // configured below the interval. Anything beyond is structurally non-finalizable.
            let max_catchup = MaxCatchup::<T>::get(chain_key) as u64;
            let attestation_interval = Pallet::<T>::chain_attestation_interval(chain_key);
            let max_roots = max_catchup.max(attestation_interval) as usize;
            if attestation.continuity_proof.len() > max_roots {
                return Err(TransactionValidityError::Invalid(
                    InvalidTransaction::Custom(OVERSIZED_PROOF_CODE),
                ));
            }

            let active_attestors = ActiveAttestors::<T>::get(chain_key)
                .into_iter()
                .collect::<BTreeSet<_>>();
            // `check_duplicate` now also enforces the strictly-monotonic per-chain height
            // (via `LastDigest`), so a race loser resubmitting an already-attested height — or
            // a quorum trying a competing digest for that height — is rejected here before fees.
            if active_attestors.contains(who) && Pallet::<T>::check_duplicate(attestation) {
                return Err(TransactionValidityError::Invalid(InvalidTransaction::Stale));
            }

            // Tag the transaction with what it provides: the attestation itself. Two pending
            // `commit_attestation` submissions for the same `(chain_key, digest)` then conflict
            // in the pool and only one is kept, deduplicating the every-attestor-submits race at
            // admission time instead of letting all copies broadcast and race to dispatch.
            return Ok(ValidTransaction {
                provides: sp_std::vec![(Self::IDENTIFIER, chain_key, attestation.digest()).encode()],
                ..Default::default()
            });
        }

        Ok((ValidTransaction::default(), (), origin))
    }

    // No implicit weight cost beyond a couple of reads that also happen at dispatch, and nothing to
    // prepare: `weight` defaults to `Weight::zero()` and `prepare` to `Ok(())`.
    impl_tx_ext_default!(<T as frame_system::Config>::RuntimeCall; weight prepare);
}

/// `InvalidTransaction::Custom` code returned when `commit_attestation` carries a continuity
/// proof exceeding `max(max_catchup, attestation_interval)` roots. Distinct from
/// `Stale`/`BadProof` so downstream tooling can disambiguate resource-exhaustion attempts from
/// race-loser duplicates.
pub const OVERSIZED_PROOF_CODE: u8 = 1;
