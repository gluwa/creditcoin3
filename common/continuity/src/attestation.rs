use sp_core::H256;

/// Minimal public representation of attestation boundary info.
#[derive(Debug, Clone)]
pub struct AttestationInfo {
    pub block_number: u64,
    pub digest: H256,
}
