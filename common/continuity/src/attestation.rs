use sp_core::H256;

/// Minimal public representation of attestation boundary info.
#[derive(Debug, Clone)]
pub struct AttestationInfo {
    pub block_number: u64,
    pub digest: H256,
    /// Previous digest from the attestation (if available)
    /// This is the digest of the block before this attestation's block number
    pub prev_digest: Option<H256>,
}
