//! Pallet Attestation POC Benchmarks
use super::Pallet as Attestation;
use super::*;
use bls_signatures::{aggregate, key::Serialize, PrivateKey};
use continuity_dev::construct_fragment;
use frame_benchmarking::v2::*;
use frame_support::assert_ok;
use frame_support::traits::{OnInitialize, OriginTrait};
use sp_core::H256;
use sp_runtime::traits::{Bounded, One};
use sp_std::{ops::RangeInclusive, vec::Vec};

use attestor_primitives::{
    attestation_fragment::AttestationFragmentSerializable, block::Block, AttestationCheckpoint,
    AttestationData as AttestationPrimitive, BlsPublicKey, BlsSignature,
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
    let balance = BalanceOf::<T>::from(1_000_000_000_000_000_000_000u128); // 1_000 units

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
    start_block: u64,
    header_number: u64,
    prev_digest: Option<H256>,
) -> SignedAttestation<<T as frame_system::Config>::Hash, <T as frame_system::Config>::AccountId> {
    let fragment = construct_fragment(
        prev_digest,
        RangeInclusive::new(start_block, header_number.saturating_sub(1)),
    );

    let attestation = AttestationPrimitive::<<T as frame_system::Config>::Hash> {
        chain_key,
        header_number,
        header_hash: <T as frame_system::Config>::Hash::default(),
        root: H256::from([0; 32]),
        prev_digest: fragment.head().map(|h| {
            let block: Block = h.clone();
            block.digest()
        }),
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

    let continuity_proof = AttestationFragmentSerializable::from(&fragment);

    let attestation = SignedAttestation {
        attestation,
        signature: aggregated_signature.as_bytes()[..]
            .try_into()
            .expect("Failed to convert to array"),
        attestors: attestors
            .iter()
            .map(|a| a.attestor_id.clone())
            .collect::<Vec<_>>(),
        continuity_proof,
    };

    attestation
}

#[benchmarks]
mod benchmarks {
    use super::*;

    pub const MAX_SPAN: u32 = 500; // continuity blocks: 10–500 for realistic weight scaling
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
    fn set_max_catchup() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let max_catchup: u32 = 10;

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            DEV_CHAIN_KEY,
            max_catchup,
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
        > = create_signed_attestation::<T>(attestors, chain_key, 1, 0, None);

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            attestation,
        )
    }

    #[benchmark]
    fn commit_attestation(
        s: Linear<10, MAX_SPAN>,     // continuity length (#headers), 10–500 blocks
        m: Linear<1, MAX_ATTESTORS>, // number of attestors
    ) {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();

        log::info!("Benchmark parameters: s = {s}, m = {m}");

        // Ensure pallet limits allow `m` attestors
        assert_ok!(Attestation::<T>::set_max_attestors(
            root_origin.clone(),
            DEV_CHAIN_KEY,
            MAX_ATTESTORS + 5 // Leave extra room in case of pre-existing attestors from mock
        ));

        // Set checkpoint interval to 10 (production-realistic: checkpoint every 100 blocks
        // with attestation_interval=10). With s=500, this creates ~5 checkpoints.
        // This ensures the benchmark reflects realistic checkpoint creation weight.
        assert_ok!(Attestation::<T>::set_attestations_per_checkpoint(
            root_origin.clone(),
            DEV_CHAIN_KEY,
            10,
        ));

        // Set target sample to one
        TargetSampleSize::<T>::set(DEV_CHAIN_KEY, 1);

        // Creating attestor to attest
        let mut attestors: Vec<Attestor<T>> = Vec::new();
        for j in 0..=m {
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

        // Creating previous attestation
        let attestation_prev: SignedAttestation<
            <T as frame_system::Config>::Hash,
            <T as frame_system::Config>::AccountId,
        > = create_signed_attestation::<T>(attestors.clone(), DEV_CHAIN_KEY, 1, 0, None);

        let attestor_origin = attestors[0].attestor_origin.clone();

        Attestation::<T>::do_start_election(2, [0; 32]).unwrap();
        assert_ok!(Attestation::<T>::commit_attestation(
            attestor_origin.clone(),
            attestation_prev.clone(),
        ));

        // Round s down to nearest 10 to reduce benchmark iterations (10, 20, 30, ... 500).
        // Continuity proof has att_header - 1 blocks.
        let s_rounded = (s / 10 * 10).max(10) as u64;
        let start_header = 1;
        let att_header = s_rounded + 1;

        log::info!(
            "Creating attestation for header {att_header}  with {} attestors",
            attestors.len()
        );

        let attestation = create_signed_attestation::<T>(
            attestors.clone(),
            DEV_CHAIN_KEY,
            start_header,
            att_header,
            Some(attestation_prev.digest()),
        );

        log::info!(
            "Created attestation for header {att_header} with digest {:?} and continuity len {}",
            attestation.digest(),
            attestation.continuity_proof.len()
        );

        #[extrinsic_call]
        _(
            attestor_origin as <T as frame_system::Config>::RuntimeOrigin,
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
            DEV_CHAIN_KEY,
            new_min_bond_requirement,
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
            attestor_id.clone(),
        ));

        assert_ok!(Attestation::<T>::attest(
            attestor.attestor_origin.clone(),
            DEV_CHAIN_KEY,
            attestor.public_key,
            attestor.signature,
        ));

        let signed_origin = attestor.stash_origin;

        #[extrinsic_call]
        _(
            signed_origin as <T as frame_system::Config>::RuntimeOrigin,
            DEV_CHAIN_KEY,
            attestor_id,
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
    fn import_checkpoints() {
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let mock_checkpoints: Vec<AttestationCheckpoint> = (0..100u8)
            .map(|i| AttestationCheckpoint {
                block_number: i as u64,
                digest: H256::from([i; 32]),
            })
            .collect();

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            DEV_CHAIN_KEY,
            mock_checkpoints.try_into().unwrap(),
        )
    }

    #[benchmark]
    fn set_attestation_chain_genesis_block_number() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let genesis_block_number: u64 = 100;

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            DEV_CHAIN_KEY,
            genesis_block_number,
        )
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
                Checkpoints::<T>::insert(chain_key, j as u64 * 100, checkpoint_digest);
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

    #[benchmark]
    fn set_election_policy() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let election_policiy = AttestorElectionPolicy::AuthorizedOnly;

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            DEV_CHAIN_KEY,
            election_policiy,
        )
    }

    #[benchmark]
    fn authorize_attestor() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();

        let stash_id = create_funded_user_with_balance::<T>("stash", 0);
        let attestor_id: T::AccountId = create_funded_user_with_balance::<T>("attestor", 4);
        let att = Attestor::<T>::new(stash_id, attestor_id.clone());

        Attestation::<T>::register_attestor(
            att.stash_origin.clone(),
            DEV_CHAIN_KEY,
            attestor_id.clone(),
        ).expect("If adding the attestor doesn't work, then we aren't benchmarking the right path anyways.");

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            DEV_CHAIN_KEY,
            attestor_id,
        )
    }

    #[benchmark]
    fn remove_authorized_attestor() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();

        let stash_id = create_funded_user_with_balance::<T>("stash", 0);
        let attestor_id: T::AccountId = create_funded_user_with_balance::<T>("attestor", 4);
        let att = Attestor::<T>::new(stash_id, attestor_id.clone());

        Attestation::<T>::register_attestor(
            att.stash_origin.clone(),
            DEV_CHAIN_KEY,
            attestor_id.clone(),
        ).expect("If adding the attestor doesn't work, then we aren't benchmarking the right path anyways.");

        Attestation::<T>::authorize_attestor(
            root_origin.clone(),
            DEV_CHAIN_KEY,
            attestor_id.clone(),
        ).expect("If authorizing the attestor doesn't work, then we aren't benchmarking the right path anyways.");

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            DEV_CHAIN_KEY,
            attestor_id,
        )
    }

    #[benchmark]
    fn kick_active_attestor() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();

        let stash_id = create_funded_user_with_balance::<T>("stash", 0);
        let attestor_id: T::AccountId = create_funded_user_with_balance::<T>("attestor", 4);
        let att = Attestor::<T>::new(stash_id, attestor_id.clone());

        Attestation::<T>::register_attestor(
            att.stash_origin.clone(),
            DEV_CHAIN_KEY,
            attestor_id.clone(),
        ).expect("If adding the attestor doesn't work, then we aren't benchmarking the right path anyways.");

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            DEV_CHAIN_KEY,
            attestor_id,
            true,
        )
    }

    #[benchmark]
    fn force_election() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();

        for j in 0..5 {
            let stash_id = create_funded_user_with_balance::<T>("stash", j);
            let attestor_id: T::AccountId =
                create_funded_user_with_balance::<T>("attestor", j + 100);
            let attestor = Attestor::<T>::new(stash_id, attestor_id.clone());

            assert_ok!(Attestation::<T>::register_attestor(
                attestor.stash_origin.clone(),
                DEV_CHAIN_KEY,
                attestor_id.clone(),
            ));

            assert_ok!(Attestation::<T>::attest(
                attestor.attestor_origin.clone(),
                DEV_CHAIN_KEY,
                attestor.public_key,
                attestor.signature,
            ));
        }

        let epoch: u64 = 1;

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            epoch,
        )
    }

    #[benchmark]
    fn force_apply_updates() {
        // Setup: populate pending storage items for all supported chains
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();

        // Set pending values that will be applied
        PendingAttestationInterval::<T>::insert(DEV_CHAIN_KEY, 200u64);
        PendingTargetSampleSize::<T>::insert(DEV_CHAIN_KEY, 10u32);
        PendingMaxCatchup::<T>::insert(DEV_CHAIN_KEY, 50u32);

        #[extrinsic_call]
        _(root_origin as <T as frame_system::Config>::RuntimeOrigin)
    }
}
