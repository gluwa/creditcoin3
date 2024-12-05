use attestor_primitives::{ChainId, ChainKey};
use sp_std::vec::Vec;

use crate::SupportedChain;

pub trait SupportedChainsProvider {
    fn is_chain_supported(chain_key: ChainKey) -> bool;
    fn supported_chains() -> Vec<ChainKey>;
    fn chain_key_by_chain_id_and_name(chain_id: ChainId, chain_name: Vec<u8>) -> Option<ChainKey>;
    fn get_supported_chain(chain_key: ChainKey) -> Option<SupportedChain>;
}
