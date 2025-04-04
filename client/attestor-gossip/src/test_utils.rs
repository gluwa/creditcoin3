use attestation_chain::attestation_fragment::AttestationFragmentSerializable;
use attestor_primitives::{Attestation as AttestationPrimitive, BlsPublicKey};
use bls_signatures::Signature;
use bls_signatures::{key::Serialize, PrivateKey};
use sp_core::sr25519::Pair as Sr25519Pair;
use sp_core::{Pair, H256};
use sp_runtime::AccountId32;

use crate::communication::Attestation;

pub struct Attestor {
    pair: Sr25519Pair,
    pub account_id: AccountId32,
    private_key: PrivateKey,
    _public_key: BlsPublicKey,
}

impl Attestor {
    pub fn new() -> Self {
        let pair: Sr25519Pair = sp_core::Pair::generate().0;
        let account_id = AccountId32::new(pair.public().0);

        let rng = sp_core::H256::random().0;
        let private_key = PrivateKey::new(rng);
        let _public_key = private_key.public_key().as_bytes()[..].try_into().unwrap();

        Self {
            pair,
            account_id,
            private_key,
            _public_key,
        }
    }

    pub fn sign_bls_attestation(&self, attestation: &AttestationPrimitive<H256>) -> Signature {
        self.private_key.sign(attestation.serialize())
    }
}

pub fn simulate_attestation_data(chain_key: u64, header_number: u64) -> AttestationPrimitive<H256> {
    AttestationPrimitive {
        chain_key,
        header_hash: H256::random(),
        header_number,
        prev_digest: None,
        root: H256::random().0,
    }
}

pub fn create_signed_attestation(
    attestor: &Attestor,
    attestation_data: AttestationPrimitive<H256>,
) -> Attestation<H256, AccountId32> {
    let bls_signature = attestor.sign_bls_attestation(&attestation_data);

    // sign attestation data
    let sr_signature = attestor.pair.sign(&attestation_data.serialize());

    Attestation {
        attestation_data,
        attestor: attestor.account_id.clone(),
        continuity_proof: AttestationFragmentSerializable::default(),
        proof_of_inclusion: Default::default(),
        signature: sr_signature,
        signature_bls: attestor_primitives::bls::WrapEncode(bls_signature),
    }
}
