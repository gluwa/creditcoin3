//! Guards for accounts nobody can legitimately own, and for the EVM address they could act as.

use core::{fmt::Debug, marker::PhantomData};

use frame_support::traits::OriginTrait;
use frame_system::RawOrigin;
use hex_literal::hex;
use pallet_evm::{EnsureAddressOrigin, EnsureAddressTruncated};
use parity_scale_codec::{Decode, DecodeWithMemTracking, Encode};
use scale_info::TypeInfo;
use sp_core::H160;
use sp_runtime::{
    impl_tx_ext_default,
    traits::{DispatchInfoOf, TransactionExtension, ValidateResult},
    transaction_validity::{InvalidTransaction, TransactionSource, ValidTransaction},
    AccountId32, Weight,
};

/// Every 32-byte encoding of an ed25519 small-order point (8 canonical, 6 non-canonical). The
/// runtime's ZIP-215 verification accepts one fixed signature for any message under these keys.
const SMALL_ORDER_KEYS: [[u8; 32]; 14] = [
    hex!("0100000000000000000000000000000000000000000000000000000000000000"),
    hex!("0100000000000000000000000000000000000000000000000000000000000080"),
    hex!("ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f"),
    hex!("ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
    hex!("0000000000000000000000000000000000000000000000000000000000000000"),
    hex!("0000000000000000000000000000000000000000000000000000000000000080"),
    hex!("c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac037a"),
    hex!("c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac03fa"),
    hex!("26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc05"),
    hex!("26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc85"),
    hex!("edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f"),
    hex!("edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
    hex!("eeffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f"),
    hex!("eeffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
];

const EXTENSION_IDENTIFIER: &str = "CheckSignerKey";

/// Refuses signed transactions from the accounts in [`SMALL_ORDER_KEYS`]. Other origins pass.
#[derive(Encode, Decode, DecodeWithMemTracking, Clone, Eq, PartialEq, TypeInfo)]
#[scale_info(skip_type_params(T))]
pub struct CheckSignerKey<T>(PhantomData<fn(T)>);

impl<T> Default for CheckSignerKey<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> CheckSignerKey<T> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<T> Debug for CheckSignerKey<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        core::write!(f, "{EXTENSION_IDENTIFIER}")
    }
}

impl<T: frame_system::Config + Send + Sync>
    TransactionExtension<<T as frame_system::Config>::RuntimeCall> for CheckSignerKey<T>
{
    const IDENTIFIER: &'static str = EXTENSION_IDENTIFIER;
    type Implicit = ();
    type Val = ();
    type Pre = ();

    // A 32-byte comparison against 14 constants, so no weight is declared.
    fn weight(&self, _: &<T as frame_system::Config>::RuntimeCall) -> Weight {
        Weight::zero()
    }

    fn validate(
        &self,
        origin: <T as frame_system::Config>::RuntimeOrigin,
        _call: &<T as frame_system::Config>::RuntimeCall,
        _info: &DispatchInfoOf<<T as frame_system::Config>::RuntimeCall>,
        _len: usize,
        _self_implicit: Self::Implicit,
        _inherited_implication: &impl Encode,
        _source: TransactionSource,
    ) -> ValidateResult<Self::Val, <T as frame_system::Config>::RuntimeCall> {
        if let Some(who) = origin.as_signer() {
            if who.using_encoded(|id| SMALL_ORDER_KEYS.iter().any(|key| key[..] == *id)) {
                return Err(InvalidTransaction::BadSigner.into());
            }
        }
        Ok((ValidTransaction::default(), (), origin))
    }

    impl_tx_ext_default!(<T as frame_system::Config>::RuntimeCall; prepare);
}

/// [`EnsureAddressTruncated`] that never lets a signed origin act as EVM address `0x0`.
pub struct EnsureAddressTruncatedNonZero;

impl<OuterOrigin> EnsureAddressOrigin<OuterOrigin> for EnsureAddressTruncatedNonZero
where
    OuterOrigin: Into<Result<RawOrigin<AccountId32>, OuterOrigin>> + From<RawOrigin<AccountId32>>,
{
    type Success = AccountId32;

    fn try_address_origin(address: &H160, origin: OuterOrigin) -> Result<AccountId32, OuterOrigin> {
        if address.is_zero() {
            return Err(origin);
        }
        EnsureAddressTruncated::try_address_origin(address, origin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Address, Executive, Runtime, RuntimeCall, RuntimeOrigin, Signature, SignedExtra,
        UncheckedExtrinsic,
    };
    use frame_support::dispatch::DispatchInfo;
    use sp_core::{ecdsa, ed25519, sr25519, Pair};
    use sp_runtime::{
        generic::Era,
        traits::{TxBaseImplication, Verify},
        transaction_validity::TransactionValidityError,
        BuildStorage, MultiSignature,
    };

    /// R = identity, S = 0: verifies under any small-order key, for any message.
    fn forged_signature() -> ed25519::Signature {
        let mut raw = [0u8; 64];
        raw[0] = 1;
        ed25519::Signature::from_raw(raw)
    }

    fn call() -> RuntimeCall {
        RuntimeCall::System(frame_system::Call::remark { remark: vec![] })
    }

    fn validate(origin: RuntimeOrigin) -> Result<(), TransactionValidityError> {
        CheckSignerKey::<Runtime>::new()
            .validate(
                origin,
                &call(),
                &DispatchInfo::default(),
                0,
                (),
                &TxBaseImplication(()),
                TransactionSource::External,
            )
            .map(|_| ())
    }

    fn new_test_ext() -> sp_io::TestExternalities {
        let storage = frame_system::GenesisConfig::<Runtime>::default()
            .build_storage()
            .unwrap();
        sp_io::TestExternalities::new(storage)
    }

    #[test]
    fn forged_signature_verifies_under_every_listed_key() {
        new_test_ext().execute_with(|| {
            let signature = MultiSignature::Ed25519(forged_signature());
            for key in SMALL_ORDER_KEYS {
                for message in [&b""[..], b"first message", b"another message"] {
                    assert!(
                        signature.verify(message, &AccountId32::new(key)),
                        "{key:?} no longer accepts the forged signature",
                    );
                }
            }
        });
    }

    #[test]
    fn listed_keys_are_distinct() {
        for (i, a) in SMALL_ORDER_KEYS.iter().enumerate() {
            assert!(SMALL_ORDER_KEYS[i + 1..].iter().all(|b| a != b));
        }
    }

    #[test]
    fn signed_origin_from_a_listed_key_is_refused() {
        new_test_ext().execute_with(|| {
            for key in SMALL_ORDER_KEYS {
                assert_eq!(
                    validate(RuntimeOrigin::signed(AccountId32::new(key))),
                    Err(InvalidTransaction::BadSigner.into()),
                );
            }
        });
    }

    #[test]
    fn ordinary_signers_and_other_origins_pass() {
        new_test_ext().execute_with(|| {
            let ordinary = [
                ed25519::Pair::from_string("//Alice", None)
                    .unwrap()
                    .public()
                    .0,
                sr25519::Pair::from_string("//Alice", None)
                    .unwrap()
                    .public()
                    .0,
                sp_io::hashing::blake2_256(
                    &ecdsa::Pair::from_string("//Alice", None)
                        .unwrap()
                        .public()
                        .0,
                ),
            ];
            for id in ordinary {
                assert_eq!(
                    validate(RuntimeOrigin::signed(AccountId32::new(id))),
                    Ok(())
                );
            }
            assert_eq!(validate(RuntimeOrigin::root()), Ok(()));
            assert_eq!(validate(RuntimeOrigin::none()), Ok(()));
        });
    }

    #[test]
    fn extension_is_wired_into_the_transaction_pipeline() {
        new_test_ext().execute_with(|| {
            let extra: SignedExtra = (
                frame_system::CheckNonZeroSender::<Runtime>::new(),
                CheckSignerKey::<Runtime>::new(),
                frame_system::CheckSpecVersion::<Runtime>::new(),
                frame_system::CheckTxVersion::<Runtime>::new(),
                frame_system::CheckGenesis::<Runtime>::new(),
                frame_system::CheckMortality::<Runtime>::from(Era::immortal()),
                frame_system::CheckNonce::<Runtime>::from(0),
                frame_system::CheckWeight::<Runtime>::new(),
                pallet_transaction_payment::ChargeTransactionPayment::<Runtime>::from(0),
                frame_metadata_hash_extension::CheckMetadataHash::new(false),
                pallet_attestation::PrevalidateAttestationCommit::<Runtime>::new(),
                frame_system::WeightReclaim::<Runtime>::new(),
            );
            // The forged signature is valid for any payload, so `extra` needs no real signing.
            let tx = UncheckedExtrinsic::new_signed(
                call(),
                // 0x00..0080: the key whose first 20 bytes are the zero EVM address.
                Address::Id(AccountId32::new(SMALL_ORDER_KEYS[5])),
                Signature::from(forged_signature()),
                extra,
            );

            assert_eq!(
                Executive::validate_transaction(
                    TransactionSource::External,
                    tx,
                    Default::default()
                ),
                Err(InvalidTransaction::BadSigner.into()),
            );
        });
    }

    #[test]
    fn evm_origin_refuses_the_zero_address_only() {
        type Guard = EnsureAddressTruncatedNonZero;
        let zero_prefixed = AccountId32::new(SMALL_ORDER_KEYS[5]);
        let ordinary = AccountId32::new([0x5A; 32]);
        let ordinary_address = H160::from_slice(&[0x5A; 20]);

        assert!(
            Guard::try_address_origin(&H160::zero(), RuntimeOrigin::signed(zero_prefixed)).is_err()
        );
        assert_eq!(
            Guard::try_address_origin(&ordinary_address, RuntimeOrigin::signed(ordinary.clone()))
                .ok(),
            Some(ordinary.clone()),
        );
        assert!(
            Guard::try_address_origin(&H160::repeat_byte(1), RuntimeOrigin::signed(ordinary))
                .is_err()
        );
    }
}
