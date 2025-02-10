//! Pallet Attestation POC Benchmarks
use super::Pallet as Attestation;
use super::*;
use bls_signatures::{aggregate, key::Serialize, PrivateKey};
use frame_benchmarking::v2::*;
use frame_support::assert_ok;
use frame_support::traits::{OnInitialize, OriginTrait};
use sp_core::H256;
use sp_runtime::traits::{Bounded, One};
use sp_std::vec;
use sp_std::vec::Vec;

use attestor_primitives::{
    Attestation as AttestationPrimitive, AttestationCheckpoint, BlsPublicKey, BlsSignature,
    ChainAttestationIntervalType, ChainKey, SignedAttestation,
};

const DEV_CHAIN_KEY: ChainKey = 1;
const SEED: u32 = 0;

#[derive(Debug, Clone)]
pub struct Attestor<T: frame_system::Config> {
    pub stash_origin: T::RuntimeOrigin,
    pub attestor_id: T::AccountId,
    pub attestor_origin: T::RuntimeOrigin,
    pub private_key: PrivateKey,
    pub public_key: BlsPublicKey,
    pub signature: BlsSignature,
}

/// Grab a funded user with max Balance.
pub fn create_funded_user_with_balance<T: Config>(string: &'static str, n: u32) -> T::AccountId {
    let balance = BalanceOf::<T>::try_from(900_000_000_000_000u128)
        .map_err(|_| "balance expected to be a u128")
        .unwrap();

    let user = account(string, n, SEED);
    asset::set_free_balance::<T>(&user, balance);
    user
}

impl<T: frame_system::Config> Attestor<T> {
    pub fn new(stash: T::AccountId, attestor: T::AccountId) -> Self {
        let rng = H256::repeat_byte(123).0;
        let private_key = PrivateKey::new(rng);
        let public_key = private_key.public_key().as_bytes()[..].try_into().unwrap();
        let signature = private_key.sign(public_key).as_bytes()[..]
            .try_into()
            .unwrap();

        let stash_origin = T::RuntimeOrigin::signed(stash);

        let attestor_id = attestor.clone();
        let attestor_origin = T::RuntimeOrigin::signed(attestor);

        Self {
            stash_origin,
            attestor_id,
            attestor_origin,
            private_key,
            public_key,
            signature,
        }
    }

    fn sign(&self, message: &[u8]) -> BlsSignature {
        self.private_key.sign(message).as_bytes()[..]
            .try_into()
            .unwrap()
    }
}

fn create_signed_attestation<T: frame_system::Config>(
    attestors: Vec<Attestor<T>>,
    chain_key: ChainKey,
    header_number: u64,
    prev_digest: Option<H256>,
) -> SignedAttestation<<T as frame_system::Config>::Hash, <T as frame_system::Config>::AccountId> {
    let attestation = AttestationPrimitive::<<T as frame_system::Config>::Hash> {
        chain_key,
        header_number,
        header_hash: <T as frame_system::Config>::Hash::default(),
        root: [0; 32],
        prev_digest,
    };

    let mut signatures = Vec::new();
    for attestor in attestors.iter() {
        let signature = attestor.sign(&attestation.serialize());
        let bls_sig = bls_signatures::Signature::from_bytes(&signature[..])
            .expect("Failed to create signature");

        signatures.push(bls_sig);
    }
    // sign
    let aggregated_signature = aggregate(&signatures).expect("Failed to aggregate signatures");

    let attestation = SignedAttestation {
        attestation,
        signature: aggregated_signature.as_bytes()[..]
            .try_into()
            .expect("Failed to convert to array"),
        attestors: attestors
            .iter()
            .map(|a| a.attestor_id.clone())
            .collect::<Vec<_>>(),
    };

    attestation
}

#[benchmarks]
mod benchmarks {
    use super::*;

    /// We want to test attestations signed by varying numbers of attestors
    const MAX_ATTESTORS: u32 = 100;

    #[benchmark]
    fn set_chain_attestation_interval() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let chain_attestation_interval: ChainAttestationIntervalType = 100;

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            DEV_CHAIN_KEY,
            chain_attestation_interval,
        )
    }

    #[benchmark]
    fn set_attestations_per_checkpoint() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let attestations_per_checkpoint: u32 = 100;

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            DEV_CHAIN_KEY,
            attestations_per_checkpoint,
        )
    }

    #[benchmark]
    fn set_target_sample_size() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let set_size: u32 = 6;

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            DEV_CHAIN_KEY,
            set_size,
        )
    }

    #[benchmark]
    fn register_attestor() {
        // Setup
        let stash_id = create_funded_user_with_balance::<T>("stash", 0);
        let attestor_id: T::AccountId = create_funded_user_with_balance::<T>("attestor", 4);

        let att = Attestor::<T>::new(stash_id, attestor_id.clone());
        let signed_origin = att.stash_origin;

        #[extrinsic_call]
        _(
            signed_origin as <T as frame_system::Config>::RuntimeOrigin,
            DEV_CHAIN_KEY,
            attestor_id,
        )
    }

    #[benchmark]
    fn unregister_attestor() {
        // Setup
        let stash_id = create_funded_user_with_balance::<T>("stash", 0);
        let attestor_id: T::AccountId = create_funded_user_with_balance::<T>("attestor", 4);
        let att = Attestor::<T>::new(stash_id, attestor_id.clone());

        Attestation::<T>::register_attestor(
            att.stash_origin.clone(),
             DEV_CHAIN_KEY,
            attestor_id.clone(),
        ).expect("If adding the attestor doesn't work, then we aren't benchmarking the right path anyways.");
        let signed_origin = att.stash_origin;

        #[extrinsic_call]
        _(
            signed_origin as <T as frame_system::Config>::RuntimeOrigin,
            DEV_CHAIN_KEY,
            attestor_id,
        )
    }

    #[benchmark]
    fn set_max_attestors() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let new_max: u32 = 20;

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            DEV_CHAIN_KEY,
            new_max,
        )
    }

    #[benchmark]
    fn register_invulnerable() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let attestor_id: <T as frame_system::Config>::AccountId = account("who", 4, 1);

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            DEV_CHAIN_KEY,
            attestor_id,
        )
    }

    #[benchmark]
    fn unregister_invulnerable() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let attestor_id: <T as frame_system::Config>::AccountId = account("who", 4, 1);

        assert_ok!(Attestation::<T>::register_invulnerable(
            root_origin.clone(),
            DEV_CHAIN_KEY,
            attestor_id.clone(),
        ));

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            DEV_CHAIN_KEY,
            attestor_id,
        )
    }

    #[benchmark]
    fn set_max_invulnerables() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let new_max: u32 = 8;

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            DEV_CHAIN_KEY,
            new_max,
        )
    }

    #[benchmark]
    fn bootstrap_chain(a: Linear<1, MAX_ATTESTORS>) {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let chain_key: ChainKey = 1;

        // Set max attestors to accomodate benchmark
        assert_ok!(Attestation::<T>::set_max_attestors(
            root_origin.clone(),
            DEV_CHAIN_KEY,
            MAX_ATTESTORS + 5 // Leave extra room in case of pre-existing attestors from mock
        ));

        // Creating attestor to attest
        let mut attestors: Vec<Attestor<T>> = Vec::new();
        for j in 1..=a {
            let stash_id = create_funded_user_with_balance::<T>("stash", j);
            let attestor_id: T::AccountId = create_funded_user_with_balance::<T>("attestor", j + j);

            let attestor = Attestor::<T>::new(stash_id, attestor_id.clone());

            assert_ok!(Attestation::<T>::register_attestor(
                attestor.stash_origin.clone(),
                DEV_CHAIN_KEY,
                attestor_id,
            ));

            assert_ok!(Attestation::<T>::attest(
                attestor.attestor_origin.clone(),
                DEV_CHAIN_KEY,
                attestor.public_key,
                attestor.signature,
            ));

            attestors.push(attestor);
        }

        let attestation: SignedAttestation<
            <T as frame_system::Config>::Hash,
            <T as frame_system::Config>::AccountId,
        > = create_signed_attestation::<T>(attestors, chain_key, 1, None);

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            attestation,
        )
    }

    #[benchmark]
    fn commit_attestation(a: Linear<1, MAX_ATTESTORS>) {
        // Setup
        let none_origin = <T as frame_system::Config>::RuntimeOrigin::none();

        // Set max attestors to accomodate benchmark
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        assert_ok!(Attestation::<T>::set_max_attestors(
            root_origin,
            DEV_CHAIN_KEY,
            MAX_ATTESTORS + 5 // Leave extra room in case of pre-existing attestors from mock
        ));

        // Creating attestor to attest
        let mut attestors: Vec<Attestor<T>> = Vec::new();
        for j in 1..=a {
            let stash_id = create_funded_user_with_balance::<T>("stash", j);
            let attestor_id: T::AccountId = create_funded_user_with_balance::<T>("attestor", j + j);
            let attestor = Attestor::<T>::new(stash_id, attestor_id.clone());

            assert_ok!(Attestation::<T>::register_attestor(
                attestor.stash_origin.clone(),
                DEV_CHAIN_KEY,
                attestor_id,
            ));

            assert_ok!(Attestation::<T>::attest(
                attestor.attestor_origin.clone(),
                DEV_CHAIN_KEY,
                attestor.public_key,
                attestor.signature,
            ));

            attestors.push(attestor);
        }

        // Create prior attestation. Needed to get most expensive code path in commit_attestation()
        let prior_attestation: SignedAttestation<
            <T as frame_system::Config>::Hash,
            <T as frame_system::Config>::AccountId,
        > = create_signed_attestation::<T>(attestors.clone(), DEV_CHAIN_KEY, 1, None);

        Attestation::<T>::do_start_election(2, [0; 32]).unwrap();

        assert_ok!(Attestation::<T>::commit_attestation(
            none_origin.clone(),
            prior_attestation.clone()
        ));

        // Create attestation
        let attestation: SignedAttestation<
            <T as frame_system::Config>::Hash,
            <T as frame_system::Config>::AccountId,
        > = create_signed_attestation::<T>(
            attestors,
            DEV_CHAIN_KEY,
            11_u64,
            Some(prior_attestation.digest()),
        );

        #[extrinsic_call]
        _(
            none_origin as <T as frame_system::Config>::RuntimeOrigin,
            attestation,
        )
    }

    #[benchmark]
    fn set_min_bond_requirement() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let new_min_bond_requirement: BalanceOf<T> = BalanceOf::<T>::max_value();

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            new_min_bond_requirement,
        )
    }

    #[benchmark]
    fn set_chain_reward() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let new_reward: BalanceOf<T> = BalanceOf::<T>::max_value();

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            DEV_CHAIN_KEY,
            new_reward,
        )
    }

    #[benchmark]
    fn attest() {
        // Setup
        let stash_id = create_funded_user_with_balance::<T>("stash", 0);
        let attestor_id: T::AccountId = create_funded_user_with_balance::<T>("attestor", 1);
        let attestor = Attestor::<T>::new(stash_id, attestor_id.clone());

        assert_ok!(Attestation::<T>::register_attestor(
            attestor.stash_origin.clone(),
            DEV_CHAIN_KEY,
            attestor_id,
        ));

        let signed_origin = attestor.attestor_origin;

        #[extrinsic_call]
        _(
            signed_origin as <T as frame_system::Config>::RuntimeOrigin,
            DEV_CHAIN_KEY,
            attestor.public_key,
            attestor.signature,
        )
    }

    #[benchmark]
    fn chill() {
        // Setup
        let stash_id = create_funded_user_with_balance::<T>("stash", 0);
        let attestor_id: T::AccountId = create_funded_user_with_balance::<T>("attestor", 1);
        let attestor = Attestor::<T>::new(stash_id, attestor_id.clone());

        assert_ok!(Attestation::<T>::register_attestor(
            attestor.stash_origin.clone(),
            DEV_CHAIN_KEY,
            attestor_id,
        ));

        assert_ok!(Attestation::<T>::attest(
            attestor.attestor_origin.clone(),
            DEV_CHAIN_KEY,
            attestor.public_key,
            attestor.signature,
        ));

        let signed_origin = attestor.attestor_origin;

        #[extrinsic_call]
        _(
            signed_origin as <T as frame_system::Config>::RuntimeOrigin,
            DEV_CHAIN_KEY,
        )
    }

    #[benchmark]
    fn set_payee() {
        // Setup
        let stash_id = create_funded_user_with_balance::<T>("stash", 0);
        let attestor_id: T::AccountId = create_funded_user_with_balance::<T>("attestor", 1);
        let attestor = Attestor::<T>::new(stash_id, attestor_id.clone());

        assert_ok!(Attestation::<T>::register_attestor(
            attestor.stash_origin.clone(),
            DEV_CHAIN_KEY,
            attestor_id,
        ));

        let signed_origin = attestor.stash_origin;

        let new_payee: <T as frame_system::Config>::AccountId = account("who", 5, 0);

        #[extrinsic_call]
        _(
            signed_origin as <T as frame_system::Config>::RuntimeOrigin,
            RewardDestination::Account(new_payee),
        )
    }

    #[benchmark]
    fn withdraw_unbonded() {
        // Setup
        let stash_id = create_funded_user_with_balance::<T>("stash", 0);
        let attestor_id: T::AccountId = create_funded_user_with_balance::<T>("attestor", 1);
        let attestor = Attestor::<T>::new(stash_id, attestor_id.clone());

        assert_ok!(Attestation::<T>::register_attestor(
            attestor.stash_origin.clone(),
            DEV_CHAIN_KEY,
            attestor_id.clone(),
        ));

        assert_ok!(Attestation::<T>::unregister_attestor(
            attestor.stash_origin.clone(),
            DEV_CHAIN_KEY,
            attestor_id,
        ));

        let signed_origin = attestor.stash_origin;

        #[extrinsic_call]
        _(signed_origin as <T as frame_system::Config>::RuntimeOrigin)
    }

    #[benchmark]
    fn claim_rewards() {
        let none_origin = <T as frame_system::Config>::RuntimeOrigin::none();

        // Setup
        let stash_id = create_funded_user_with_balance::<T>("stash", 0);
        let attestor_id: T::AccountId = create_funded_user_with_balance::<T>("attestor", 1);
        let attestor = Attestor::<T>::new(stash_id, attestor_id.clone());

        assert_ok!(Attestation::<T>::register_attestor(
            attestor.stash_origin.clone(),
            DEV_CHAIN_KEY,
            attestor_id,
        ));

        assert_ok!(Attestation::<T>::attest(
            attestor.attestor_origin.clone(),
            DEV_CHAIN_KEY,
            attestor.public_key,
            attestor.signature,
        ));

        // create single attestation
        let attestation: SignedAttestation<
            <T as frame_system::Config>::Hash,
            <T as frame_system::Config>::AccountId,
        > = create_signed_attestation::<T>(vec![attestor.clone()], DEV_CHAIN_KEY, 1, None);

        Attestation::<T>::do_start_election(2, [0; 32]).unwrap();

        assert_ok!(Attestation::<T>::commit_attestation(
            none_origin.clone(),
            attestation.clone(),
        ));

        let signed_origin = attestor.stash_origin;

        #[extrinsic_call]
        _(signed_origin as <T as frame_system::Config>::RuntimeOrigin)
    }

    #[benchmark]
    fn on_initialize(a: Linear<0, 1>) {
        frame_system::Pallet::<T>::set_block_number(One::one());
        // Set up 0-1 chains with checkpoints to be removed. Should add at least
        // MAX_CHECKPOINTS_CLEARED_PER_BLOCK attestations to ensure appropriately
        // pessemistic weight.
        if a == 1 {
            let chain_key = 5;
            for j in 0..(MAX_CHECKPOINTS_CLEARED_PER_BLOCK * 2 + 10) {
                let checkpoint_digest = H256::from(&sp_io::hashing::blake2_256(&[j]));
                let checkpoint = AttestationCheckpoint {
                    block_number: j as u64 * 100, // Mimic gap between checkpoint blocks
                    digest: checkpoint_digest,
                };
                Checkpoints::<T>::insert(chain_key, checkpoint_digest, checkpoint);
            }

            // Mimic the effects of on_supported_chain_removed
            let maybe_cursor = Checkpoints::<T>::clear_prefix(
                chain_key,
                u32::from(MAX_CHECKPOINTS_CLEARED_PER_BLOCK),
                None,
            )
            .maybe_cursor;
            CheckpointClearingCursors::<T>::set(chain_key, maybe_cursor);
        }

        #[block]
        {
            Attestation::<T>::on_initialize(One::one());
        }
    }
}
