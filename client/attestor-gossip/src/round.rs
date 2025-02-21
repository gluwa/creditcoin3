use parity_scale_codec::{Codec, Decode, Encode};
use sp_api::ProvideRuntimeApi;
use sp_runtime::traits::Block as BlockT;
use std::sync::Arc;

use crate::communication::Error;
use crate::HashFor;

use attestor_primitives::{api::AttestorApi, ChainKey};

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct RoundConfig {
    pub committee_set_size: u32,
    pub target_sample_size: u32,
    pub threshold: u32,
}

pub fn get_round_config<RA, B, AccountId>(
    ra: Arc<RA>,
    chain_key: ChainKey,
    block_hash: HashFor<B>,
) -> Result<RoundConfig, Error>
where
    RA: ProvideRuntimeApi<B> + Send + Sync + 'static,
    RA::Api: AttestorApi<B, HashFor<B>, AccountId>,
    B: BlockT,
    AccountId: Codec,
{
    let target_sample_size = ra.runtime_api().target_sample_size(block_hash, chain_key)?;
    let committee_set_size = ra.runtime_api().working_set_size(block_hash, chain_key)?;

    let threshold = calculate_threshold(committee_set_size);

    Ok(RoundConfig {
        committee_set_size,
        target_sample_size,
        threshold,
    })
}

/// Function to calculate the threshold for a committee set size to reach majority vote
fn calculate_threshold(committee_set_size: u32) -> u32 {
    (2 * committee_set_size) / 3
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_calculate_threshold_3() {
        let committee_set_size = 3;
        let threshold = calculate_threshold(committee_set_size);
        assert_eq!(threshold, 2);
    }

    #[test]
    fn test_calculate_threshold_4() {
        let committee_set_size = 4;
        let threshold = calculate_threshold(committee_set_size);
        assert_eq!(threshold, 2);
    }

    #[test]
    fn test_calculate_threshold_5() {
        let committee_set_size = 5;
        let threshold = calculate_threshold(committee_set_size);
        assert_eq!(threshold, 3);
    }

    #[test]
    fn test_calculate_threshold_10() {
        let committee_set_size = 10;
        let threshold = calculate_threshold(committee_set_size);
        assert_eq!(threshold, 6);
    }
}
