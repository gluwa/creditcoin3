//! Common types aliases used throughout the attestor code, either for maintainability by making it
//! easy to swap underlying types in the future, or for readability by associating a specific use
//! case to more generic types like [u64].

pub type Height = u64;
pub type Epoch = u64;

pub type SubxtClient = subxt::OnlineClient<subxt::SubstrateConfig>;
pub type SubxtBlock = subxt::blocks::Block<subxt::SubstrateConfig, SubxtClient>;
pub type SubxtBlockStream = subxt::backend::StreamOf<std::result::Result<SubxtBlock, subxt::Error>>;

pub type Attestation =
    attestor_primitives::Attestation<attestor_primitives::Digest, attestor_primitives::AttestorId>;
pub type AttestationData = attestor_primitives::AttestationData<attestor_primitives::Digest>;
pub type AttestationSigned = attestor_primitives::SignedAttestation<
    attestor_primitives::Digest,
    attestor_primitives::AttestorId,
>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttestationInfo {
    pub digest: attestor_primitives::Digest,
    pub height: Height,
}

pub(crate) type Metrics = std::sync::Arc<crate::worker::api::metrics::Metrics>;
