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

use super::*;
use attestor_primitives::ChainId;
use frame_benchmarking::v2::*;
use frame_support::traits::OriginTrait;
use sp_std::vec;
use attestor_primitives::{BlsPublicKey, BlsSignature};
use bls_signatures::{PrivateKey, key::Serialize};

#[derive(Debug, Clone)]
pub struct Attestor<T: frame_system::Config> {
    pub id: T::AccountId,
    pub attestor: T::RuntimeOrigin,
    pub private_key: PrivateKey,
    pub public_key: BlsPublicKey,
    pub signature: BlsSignature,
}

impl<T: frame_system::Config> Attestor<T> {
    pub fn new(attestor: T::AccountId) -> Self {
        let rng = sp_core::H256::repeat_byte(123).0;
        let private_key = PrivateKey::new(rng);
        let public_key = private_key.public_key().as_bytes()[..].try_into().unwrap();
        let signature = private_key.sign(public_key).as_bytes()[..]
            .try_into()
            .unwrap();

        let id = attestor.clone();
        let attestor = T::RuntimeOrigin::signed(attestor);

        Self {
            id,
            attestor,
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
        let signed_origin = att.attestor;
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
        //TODO: Might need to register attestor first to get happy path. But the storage access
        //      count may be the same either way
        let signed_origin = att.attestor;

        #[extrinsic_call]
        _(
            signed_origin as <T as frame_system::Config>::RuntimeOrigin,
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

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
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
            new_max,
        )
    }

    // Possibly use Attestor::sign() to sign attestation
}
