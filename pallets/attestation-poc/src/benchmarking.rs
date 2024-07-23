// Copyright (C) Parity Technologies (UK) Ltd.
// This file is part of Polkadot.

// Polkadot is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Polkadot is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Polkadot.  If not, see <http://www.gnu.org/licenses/>.

//! On demand assigner pallet benchmarking.

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
    fn bootstrap_chain() {
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

        // Creating attestor to attest
        let attestor = Attestor::<T>::new(account("who", 3, 0));

        assert_ok!(Attestation::<T>::register_attestor(
            attestor.origin.clone(),
            attestor.public_key.clone(),
            attestor.signature.clone()
        ));

        let attestation: SignedAttestation<
            <T as frame_system::Config>::Hash,
            <T as frame_system::Config>::AccountId,
        > = create_signed_attestation::<T>(Vec::from([attestor]), chain_id, 1, None);

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            chain_id,
            attestation,
        )
    }

    #[benchmark]
    fn commit_attestation() {
        // Setup
        let chain_id: ChainId = 1;
        let none_origin = <T as frame_system::Config>::RuntimeOrigin::none();

        // Creating attestor to attest
        let attestor = Attestor::<T>::new(account("who", 3, 0));

        assert_ok!(Attestation::<T>::register_attestor(
            attestor.origin.clone(),
            attestor.public_key.clone(),
            attestor.signature.clone()
        ));

        // Create attestation
        let attestation: SignedAttestation<
            <T as frame_system::Config>::Hash,
            <T as frame_system::Config>::AccountId,
        > = create_signed_attestation::<T>(Vec::from([attestor]), chain_id, 1, None);

        #[extrinsic_call]
        _(
            none_origin as <T as frame_system::Config>::RuntimeOrigin,
            attestation,
        )
    }
}
