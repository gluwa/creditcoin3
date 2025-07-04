use attestor_primitives::{ChainId, ChainKey};
use sp_std::vec::Vec;

use crate::SupportedChain;

pub trait SupportedChainsProvider {
    fn is_chain_supported(chain_key: ChainKey) -> bool;
    fn supported_chains() -> Vec<ChainKey>;
    fn chain_key_by_chain_id_and_name(chain_id: ChainId, chain_name: Vec<u8>) -> Option<ChainKey>;
    fn get_supported_chain(chain_key: ChainKey) -> Option<SupportedChain>;
}

pub trait OnRegisterChainProvider {
    fn on_register_chain(
        chain_key: ChainKey,
        chain_id: ChainId,
        chain_name: Vec<u8>,
        target_sample_size: Option<u32>,
        chain_attestation_interval: Option<u64>,
        attestation_checkpoint_interval: Option<u32>,
        chain_reward: Option<u128>,
        max_attestors: Option<u32>,
        max_invulnerables: Option<u32>,
        attestation_chain_genesis_block_number: Option<u64>,
    );
}
