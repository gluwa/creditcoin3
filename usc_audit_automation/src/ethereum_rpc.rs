use crate::{ChainCache, RpcProvider, SupportedChainInfo};
use anyhow::Result;
use eth::Client;
use tracing::{error, info, warn};

async fn ethereum_rpc_is_healthy(_rpc_url: &str) -> Result<bool> {
    let eth_client_result = Client::new(_rpc_url, None).await?;
    match eth_client_result.get_chain_id().await {
        Ok(_) => Ok(true),
        Err(e) => {
            error!("RPC connection error: {}", e);
            Err(anyhow::anyhow!("WSS connection error: {}", e))
        }
    }
}

fn replace_rpc_key(rpc_url: &str, new_key: &str) -> Option<String> {
    // Determine the version prefix to know where to split.
    let version_prefix = if rpc_url.contains("/v2/") {
        // Alchemy
        "/v2/"
    } else if rpc_url.contains("/v3/") {
        // Infura
        "/v3/"
    } else {
        // Handle URLs without a version path
        return Some(rpc_url.to_string());
    };

    // Find the starting position of the key (after the version prefix)
    if let Some(pos) = rpc_url.find(version_prefix) {
        let base_end = pos + version_prefix.len();

        let base_url = &rpc_url[..base_end];

        Some(format!("{base_url}{new_key}"))
    } else {
        None
    }
}

pub async fn get_ethereum_rpc_url_from_chain_cache(
    chain_cache: ChainCache,
    rpc_providers: &[RpcProvider],
    supported_chain_info: &SupportedChainInfo,
) -> Option<String> {
    if supported_chain_info.chain_id == 31337 {
        info!("Skipping RPC healthcheck for local development chain (chain ID 31337)");
        return Some("http://localhost:8545".into());
    }

    let rpc_urls: Vec<String> = chain_cache
        .get_by_id(supported_chain_info.chain_id)
        .map(|c| c.rpc.clone())
        .unwrap_or_default();

    for rpc_provider in rpc_providers.iter() {
        let provider_lower = rpc_provider.name.to_lowercase();

        if let Some(rpc_url) = rpc_urls
            .iter()
            .find(|url| url.to_lowercase().contains(&provider_lower))
        {
            if rpc_provider.api_key.is_empty() {
                info!(
                    "Skipping {} because no api key was provided.",
                    rpc_provider.name
                );
                continue;
            }

            if let Some(url) = replace_rpc_key(rpc_url, &rpc_provider.api_key) {
                info!("Testing RPC connection for {}...", rpc_provider.name);

                if ethereum_rpc_is_healthy(&url).await.is_ok() {
                    info!(
                        "RPC Healthcheck successful for {} at {}.",
                        rpc_provider.name, url
                    );
                    return Some(url);
                } else {
                    warn!(
                        "RPC Healthcheck failed for {} at {}. Trying next provider.",
                        rpc_provider.name, url
                    );
                }
            }
        }
    }

    info!("No Infura or Alchemy RPC URL found for this chain. Trying other providers...");
    for rpc_url in rpc_urls.iter() {
        if ethereum_rpc_is_healthy(rpc_url).await.is_ok() {
            info!("RPC Healthcheck successful at {}.", rpc_url);
            return Some(rpc_url.clone());
        } else {
            warn!(
                "RPC Healthcheck failed at {}. Trying next provider.",
                rpc_url
            );
        }
    }
    None
}
