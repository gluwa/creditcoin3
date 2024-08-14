use attestor_primitives::ChainId;
use sp_std::vec::Vec;

pub trait SupportedChainsProvider {
    fn is_chain_supported(chain_id: ChainId) -> bool;
    fn supported_chains() -> Option<Vec<ChainId>>;
    fn generated_key_by_chain_id_and_name(
        chain_id: ChainId,
        chain_name: Vec<u8>,
    ) -> Option<ChainId>;
}
