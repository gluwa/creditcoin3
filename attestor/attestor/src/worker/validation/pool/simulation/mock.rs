#[derive(Clone, Debug)]
pub struct Attestor {
    id: attestor_primitives::AttestorId,
    keypair: subxt_signer::sr25519::Keypair,
    bls_key: bls_signatures::PrivateKey,
}

impl Attestor {
    pub fn new(index: attestor_primitives::Height) -> Self {
        use std::str::FromStr as _;

        let seed: [u8; 32] = std::array::from_fn(|n| ((u8::MAX << n) as u64 & index) as u8);

        let key = bip39::Mnemonic::from_entropy(&[index; 32])
            .unwrap()
            .to_string();

        let secret_uri = subxt_signer::SecretUri::from_str(&key).unwrap();
        let keypair = subxt_signer::sr25519::Keypair::from_uri(&secret_uri).unwrap();
        let bls = bls_signatures::PrivateKey::new([index; 32]);

        Self {
            id: attestor_primitives::AttestorId::from_public([index; 32]),
            keypair,
            bls_key: bls,
        }
    }

    pub fn sign_attestation(
        &self,
        attestation_data: attestor_primitives::AttestationData<attestor_primitives::Digest>,
        continuity_proof: attestor_primitives::attestation_fragment::AttestationFragmentSerializable,
    ) -> attestor_primitives::Attestation<
        attestor_primitives::Digest,
        attestor_primitives::AttestorId,
    > {
        let attestor = self.id.clone();
        let message = attestation_data.serialize();
        let signature = sp_core::sr25519::Signature::from_raw(self.keypair.sign(&message).0);
        let signature_bls = attestor_primitives::bls::WrapEncode(self.bls_key.sign(message));

        attestor_primitives::Attestation {
            attestation_data,
            attestor,
            signature,
            signature_bls,
            continuity_proof,
        }
    }

    pub fn id(&self) -> attestor_primitives::AttestorId {
        self.id.clone()
    }
}
