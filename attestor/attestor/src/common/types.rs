//! Common types aliases used throughout the attestor code, either for maintainability by making it
//! easy to swap underlying types in the future, or for readability by associating a specific use
//! case to more generic types like [u64].

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub type Height = u64;
pub type Epoch = u64;

pub type SubxtClient = subxt::OnlineClient<subxt::SubstrateConfig>;
pub type SubxtBlock = subxt::blocks::Block<subxt::SubstrateConfig, SubxtClient>;
pub type SubxtBlockStream = subxt::backend::StreamOf<std::result::Result<SubxtBlock, subxt::Error>>;

pub type BlsSignature =
    <attestor_primitives::bls::Bls as attestor_primitives::bls::CryptoScheme>::Signature;

pub type AttestationSigned = attestor_primitives::SignedAttestation<
    attestor_primitives::Digest,
    attestor_primitives::AttestorId,
>;
pub type Batch = Vec<
    cc_client::cc3::runtime_types::attestor_primitives::SignedAttestation<
        cc_client::H256,
        cc_client::AccountId32,
    >,
>;

#[derive(Debug, Clone, PartialEq, Eq, parity_scale_codec::Encode, parity_scale_codec::Decode)]
pub struct Attestation {
    pub attestation_data: attestor_primitives::Attestation<attestor_primitives::Digest>,
    pub attestor: attestor_primitives::AttestorId,
    pub signature: sp_core::sr25519::Signature,
    pub signature_bls: BlsSignature,
    pub continuity_proof:
        attestor_primitives::attestation_fragment::AttestationFragmentSerializable,
    pub epoch: Epoch,
}

impl Attestation {
    pub fn digest(&self) -> attestor_primitives::Digest {
        self.attestation_data.digest()
    }

    pub fn prev_digest(&self) -> Option<attestor_primitives::Digest> {
        self.attestation_data.prev_digest()
    }

    pub fn round(&self) -> attestor_primitives::Round {
        self.attestation_data.round()
    }

    pub fn chain_key(&self) -> attestor_primitives::ChainKey {
        self.attestation_data.chain_key()
    }

    pub fn header_number(&self) -> Height {
        self.attestation_data.header_number
    }

    pub fn attestor_id(&self) -> attestor_primitives::AttestorId {
        self.attestor.clone()
    }
}
