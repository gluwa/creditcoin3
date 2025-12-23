use crate::{ChainCache, RpcProvider, SupportedChainInfo};
use anyhow::Result;
use eth::Client;
use tracing::{error, info, warn};

/// Redacts API keys from RPC URLs for safe logging
pub fn redact_api_key_from_url(url: &str) -> String {
    // Check if URL contains version paths that indicate API keys
    if let Some(pos) = url.find("/v2/").or_else(|| url.find("/v3/")) {
        let base = &url[..pos + 4]; // Include "/v2/" or "/v3/"
        format!("{base}[REDACTED]")
    } else {
        url.to_string()
    }
}

async fn ethereum_rpc_is_healthy(_rpc_url: &str) -> Result<bool> {
    let eth_client_result = Client::new(_rpc_url, None).await?;
    match eth_client_result.get_chain_id().await {
        Ok(_) => Ok(true),
        Err(e) => {
            error!("RPC connection error: {e}");
            Err(anyhow::anyhow!("WSS connection error: {e}"))
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
            let api_key = match &rpc_provider.api_key {
                Some(key) if !key.is_empty() => key,
                _ => {
                    info!(
                        "Skipping {} because no api key was provided.",
                        rpc_provider.name
                    );
                    continue;
                }
            };

            if let Some(url) = replace_rpc_key(rpc_url, api_key) {
                let provider_name = &rpc_provider.name;
                info!("Testing RPC connection for {provider_name}...");

                if ethereum_rpc_is_healthy(&url).await.is_ok() {
                    info!(
                        "RPC Healthcheck successful for {} at {}.",
                        rpc_provider.name,
                        redact_api_key_from_url(&url)
                    );
                    return Some(url);
                } else {
                    warn!(
                        "RPC Healthcheck failed for {} at {}. Trying next provider.",
                        rpc_provider.name,
                        redact_api_key_from_url(&url)
                    );
                }
            }
        }
    }

    info!("No Infura or Alchemy RPC URL found for this chain. Trying other providers...");
    for rpc_url in rpc_urls.iter() {
        if ethereum_rpc_is_healthy(rpc_url).await.is_ok() {
            info!(
                "RPC Healthcheck successful at {}.",
                redact_api_key_from_url(rpc_url)
            );
            return Some(rpc_url.clone());
        } else {
            warn!(
                "RPC Healthcheck failed at {}. Trying next provider.",
                redact_api_key_from_url(rpc_url)
            );
        }
    }
    None
}
