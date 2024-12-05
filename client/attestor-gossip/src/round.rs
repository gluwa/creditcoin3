use parity_scale_codec::{Codec, Decode, Encode};
use sp_api::ProvideRuntimeApi;
use sp_runtime::traits::Block as BlockT;
use std::{
    fmt::{Debug, Display},
    sync::Arc,
};

use crate::{Error, HashFor};

use attestor_primitives::{api::AttestorApi, ChainKey};

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct RoundConfig {
    pub committee_set_size: u32,
    pub target_sample_size: u32,
    pub threshold: u32,
    pub epoch: u64,
}

pub fn create_round_config<RA, B, AccountId>(
    ra: Arc<RA>,
    chain_key: ChainKey,
    block_hash: HashFor<B>,
    epoch: u64,
    committee_set_size: u32,
) -> Result<RoundConfig, Error>
where
    RA: ProvideRuntimeApi<B> + Send + Sync + 'static,
    RA::Api: AttestorApi<B, HashFor<B>, AccountId>,
    B: BlockT,
    AccountId: Clone
        + Display
        + Codec
        + Send
        + 'static
        + Sync
        + Debug
        + Into<[u8; 32]>
        + PartialEq
        + Eq
        + std::hash::Hash,
{
    let target_sample_size = ra.runtime_api().target_sample_size(block_hash, chain_key)?;

    let threshold = calculate_threshold(committee_set_size);

    Ok(RoundConfig {
        committee_set_size,
        target_sample_size,
        threshold,
        epoch,
    })
}

/// Function to calculate the threshold for a committee set size to reach majority vote
fn calculate_threshold(committee_set_size: u32) -> u32 {
    (2 * committee_set_size + 3) / 3
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_calculate_threshold_3() {
        let committee_set_size = 3;
        let threshold = calculate_threshold(committee_set_size);
        assert_eq!(threshold, 3);
    }

    #[test]
    fn test_calculate_threshold_4() {
        let committee_set_size = 4;
        let threshold = calculate_threshold(committee_set_size);
        assert_eq!(threshold, 3);
    }

    #[test]
    fn test_calculate_threshold_5() {
        let committee_set_size = 5;
        let threshold = calculate_threshold(committee_set_size);
        assert_eq!(threshold, 4);
    }

    #[test]
    fn test_calculate_threshold_10() {
        let committee_set_size = 10;
        let threshold = calculate_threshold(committee_set_size);
        assert_eq!(threshold, 7);
    }
}
