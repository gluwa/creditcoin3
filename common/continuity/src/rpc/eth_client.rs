#[async_trait::async_trait]
pub trait EthClientRpc {
    async fn chain_id(&self) -> Result<u64, ContinuityError>;

    async fn create_continuity_fragment(
        &self,
        start: u64,
        end: u64,
        lower_digest: H256,
    ) -> Result<Vec<Block>, ContinuityError>;
}
