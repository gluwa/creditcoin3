//! Pallet Attestation POC Benchmarks

#![cfg(feature = "runtime-benchmarks")]

use super::Pallet as Attestation;
use super::*;
use attestor_primitives::{
    Attestation as AttestationPrimitive, BlsPublicKey, BlsSignature, ChainId, SignedAttestation,
};
use bls_signatures::{aggregate, key::Serialize, PrivateKey};
use frame_benchmarking::v2::*;
use frame_support::assert_ok;
use frame_support::traits::OriginTrait;
use sp_core::H256;
use sp_std::vec;
use sp_std::vec::Vec;

#[derive(Debug, Clone)]
pub struct Attestor<T: frame_system::Config> {
    pub id: T::AccountId,
    pub origin: T::RuntimeOrigin,
    pub private_key: PrivateKey,
    pub public_key: BlsPublicKey,
    pub signature: BlsSignature,
}

impl<T: frame_system::Config> Attestor<T> {
    pub fn new(attestor: T::AccountId) -> Self {
        let rng = H256::repeat_byte(123).0;
        let private_key = PrivateKey::new(rng);
        let public_key = private_key.public_key().as_bytes()[..].try_into().unwrap();
        let signature = private_key.sign(public_key).as_bytes()[..]
            .try_into()
            .unwrap();

        let id = attestor.clone();
        let origin = T::RuntimeOrigin::signed(attestor);

        Self {
            id,
            origin,
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
    chain_id: ChainId,
    header_number: u64,
    prev_digest: Option<H256>,
) -> SignedAttestation<<T as frame_system::Config>::Hash, <T as frame_system::Config>::AccountId> {
    let attestation = AttestationPrimitive::<<T as frame_system::Config>::Hash> {
        chain_id,
        header_number,
        header_hash: <T as frame_system::Config>::Hash::default(),
        tx_root: [0; 32],
        rx_root: [0; 32],
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
        attestors: attestors.iter().map(|a| a.id.clone()).collect::<Vec<_>>(),
    };

    attestation
}

#[benchmarks]
mod benchmarks {
    use super::*;

    /// We want to test attestations signed by varying numbers of attestors
    const MAX_ATTESTORS: u32 = 100;

    #[benchmark]
    fn add_supported_chain() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let chain_attestation_interval: ChainAttestationIntervalType = 100;
        let chain_id: ChainId = 2;

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            chain_id,
            chain_attestation_interval,
        )
    }

    #[benchmark]
    fn set_comittee_set_size() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let set_size: u32 = 6;

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            set_size,
        )
    }

    #[benchmark]
    fn register_attestor() {
        // Setup
        let attestor_id: <T as frame_system::Config>::AccountId = account("who", 4, 0);
        let att = Attestor::<T>::new(attestor_id);
        let signed_origin = att.origin;
        let bls_public_key: BlsPublicKey = att.public_key;
        let proof_of_possession: BlsSignature = att.signature;

        #[extrinsic_call]
        _(
            signed_origin as <T as frame_system::Config>::RuntimeOrigin,
            bls_public_key,
            proof_of_possession,
        )
    }

    #[benchmark]
    fn unregister_attestor() {
        // Setup
        let attestor_id: <T as frame_system::Config>::AccountId = account("who", 4, 0);
        let att = Attestor::<T>::new(attestor_id);

        Attestation::<T>::register_attestor(
            att.origin.clone(),
            att.public_key,
            att.signature
        ).expect("If adding the attestor doesn't work, then we aren't benchmarking the right path anyways.");
        let signed_origin = att.origin;

        #[extrinsic_call]
        _(signed_origin as <T as frame_system::Config>::RuntimeOrigin)
    }

    #[benchmark]
    fn set_max_attestors() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let new_max: u32 = 20;

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            new_max,
        )
    }

    #[benchmark]
    fn register_invulnerable() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let attestor_id: <T as frame_system::Config>::AccountId = account("who", 4, 0);
        let att = Attestor::<T>::new(attestor_id.clone());
        let bls_public_key: BlsPublicKey = att.public_key;

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            attestor_id,
            bls_public_key,
        )
    }

    #[benchmark]
    fn unregister_invulnerable() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let attestor_id: <T as frame_system::Config>::AccountId = account("who", 3, 0);
        let att = Attestor::<T>::new(attestor_id);
        assert_ok!(Attestation::<T>::register_invulnerable(
            root_origin.clone(),
            att.id.clone(),
            att.public_key,
        ));

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            att.id,
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
            new_max,
        )
    }

    #[benchmark]
    fn bootstrap_chain(a: Linear<1, MAX_ATTESTORS>) {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let chain_id: ChainId = 2;

        // Adding new supported chain
        let chain_attestation_interval: ChainAttestationIntervalType = 100;
        assert_ok!(Attestation::<T>::add_supported_chain(
            root_origin.clone(),
            chain_attestation_interval,
            chain_id
        ));

        // Set max attestors to accomodate benchmark
        assert_ok!(Attestation::<T>::set_max_attestors(
            root_origin.clone(),
            MAX_ATTESTORS + 5 // Leave extra room in case of pre-existing attestors from mock
        ));

        // Creating attestor to attest
        let mut attestors: Vec<Attestor<T>> = Vec::new();
        for j in 1..=a {
            let attestor = Attestor::<T>::new(account("who", j, 0));

            assert_ok!(Attestation::<T>::register_attestor(
                attestor.origin.clone(),
                attestor.public_key,
                attestor.signature
            ));

            attestors.push(attestor);
        }

        let attestation: SignedAttestation<
            <T as frame_system::Config>::Hash,
            <T as frame_system::Config>::AccountId,
        > = create_signed_attestation::<T>(attestors, chain_id, 1, None);

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            chain_id,
            attestation,
        )
    }

    #[benchmark]
    fn commit_attestation(a: Linear<1, MAX_ATTESTORS>) {
        // Setup
        let chain_id: ChainId = 1;
        let none_origin = <T as frame_system::Config>::RuntimeOrigin::none();

        // Set max attestors to accomodate benchmark
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        assert_ok!(Attestation::<T>::set_max_attestors(
            root_origin,
            MAX_ATTESTORS + 5 // Leave extra room in case of pre-existing attestors from mock
        ));

        // Creating attestor to attest
        let mut attestors: Vec<Attestor<T>> = Vec::new();
        for j in 1..=a {
            let attestor = Attestor::<T>::new(account("who", j, 0));

            assert_ok!(Attestation::<T>::register_attestor(
                attestor.origin.clone(),
                attestor.public_key,
                attestor.signature
            ));

            attestors.push(attestor);
        }

        // Create prior attestation. Needed to get most expensive code path in commit_attestation()
        let prior_attestation: SignedAttestation<
            <T as frame_system::Config>::Hash,
            <T as frame_system::Config>::AccountId,
        > = create_signed_attestation::<T>(attestors.clone(), chain_id, 1, None);

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
            chain_id,
            11_u64,
            Some(prior_attestation.digest()),
        );

        #[extrinsic_call]
        _(
            none_origin as <T as frame_system::Config>::RuntimeOrigin,
            attestation,
        )
    }
}
