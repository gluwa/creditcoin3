use async_trait::async_trait;
use attestor_primitives::{AttestationCheckpoint, Digest, SignedAttestation};
use sp_core::H256;

#[async_trait]
pub trait CcRpcProvider: Send + Sync {
    async fn get_signed_attestation(
        &self,
        chain_key: u64,
        header_number: u64,
    ) -> anyhow::Result<Option<SignedAttestation<Digest, H256>>>;

    async fn get_latest_checkpoint(
        &self,
        chain_key: u64,
    ) -> anyhow::Result<Option<AttestationCheckpoint>>;

    async fn get_attestation_header_number_by_digest(
        &self,
        chain_key: u64,
        digest: Digest,
    ) -> anyhow::Result<Option<u64>>;
}
