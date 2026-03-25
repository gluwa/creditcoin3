pub const ATTESTOR_VALID_0: attestor_primitives::AttestorId =
    attestor_primitives::AttestorId::from_public(*b"attestor_valid_0________________");
pub const ATTESTOR_VALID_1: attestor_primitives::AttestorId =
    attestor_primitives::AttestorId::from_public(*b"attestor_valid_1________________");
pub const ATTESTOR_VALID_2: attestor_primitives::AttestorId =
    attestor_primitives::AttestorId::from_public(*b"attestor_valid_2________________");
pub const ATTESTOR_VALID_3: attestor_primitives::AttestorId =
    attestor_primitives::AttestorId::from_public(*b"attestor_valid_3________________");
pub const ATTESTOR_INVALID: attestor_primitives::AttestorId =
    attestor_primitives::AttestorId::from_public(*b"attestor_invalid________________");

pub const DIGEST_0: attestor_primitives::Digest =
    sp_core::H256(*b"digest_0________________________");
pub const DIGEST_1: attestor_primitives::Digest =
    sp_core::H256(*b"digest_1________________________");

pub const TIMEOUT: std::time::Duration = std::time::Duration::from_millis(10);
