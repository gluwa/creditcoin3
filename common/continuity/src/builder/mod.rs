//! ContinuityBuilder – orchestrates fetching attestations, selecting bounds,
//! generating continuity fragments, and trimming chains.

mod bounds;
mod build;
mod fetch;

pub use fetch::*;

use crate::config::ContinuityConfig;
use crate::proof::ContinuityProof;

use attestor_primitives::Query;
use cc_client::Client as CcClient;
use eth::Client as EthClient;

use anyhow::{anyhow, Result};

/// Main continuity proof builder.
pub struct ContinuityBuilder {
    pub config: ContinuityConfig,
    pub cc_client: CcClient,
    pub eth_client: EthClient,
}

impl ContinuityBuilder {
    pub async fn new(config: ContinuityConfig) -> Result<Self> {
        let cc_client = CcClient::new(&config.cc3_rpc_url, "")
            .await
            .map_err(|e| anyhow!("Failed to create CC client: {}", e))?;

        let eth_client = EthClient::new(&config.eth_rpc_url, None)
            .await
            .map_err(|e| anyhow!("Failed to create ETH client: {}", e))?;

        Ok(Self {
            config,
            cc_client,
            eth_client,
        })
    }

    pub async fn build_for_single_query(&self, query: &Query) -> Result<ContinuityProof> {
        self.build_for_heights(&[query.height]).await
    }

    pub async fn build_for_batch_queries(&self, heights: &[u64]) -> Result<ContinuityProof> {
        if heights.is_empty() {
            return Err(anyhow!("No query heights provided"));
        }
        self.build_for_heights(heights).await
    }
}
