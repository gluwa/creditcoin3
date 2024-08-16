use attestor_primitives::{ChainId, ChainKey};
use sp_std::vec::Vec;

pub trait SupportedChainsProvider {
    fn is_chain_supported(chain_id: ChainKey) -> bool;
    fn supported_chains() -> Option<Vec<ChainKey>>;
    fn chain_key_by_chain_id_and_name(chain_id: ChainId, chain_name: Vec<u8>) -> Option<ChainKey>;
}
