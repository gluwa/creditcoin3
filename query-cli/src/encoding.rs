//! Resolve the source-chain block encoding from CC3 supported-chain metadata.
//!
//! The query CLI used to hardcode [`EncodingVersion::V1`] in both the batch and
//! interactive paths. This helper instead reads the chain's configured
//! `chain_encoding` so a per-chain or future encoding change is honoured. If the
//! chain cannot be resolved we fall back to V1 (the only encoding in existence
//! today) and warn, rather than failing the query outright.

use cc_client::Client as CcClient;
use usc_abi_encoding::common::EncodingVersion;

/// Fetch the configured block encoding for `chain_key` from CC3, defaulting to
/// [`EncodingVersion::V1`] when the lookup fails or the chain is unknown.
pub(crate) async fn resolve_chain_encoding(cc3_rpc_url: &str, chain_key: u64) -> EncodingVersion {
    match CcClient::new_read_only(cc3_rpc_url).await {
        Ok(client) => match client.get_supported_chain(chain_key).await {
            Ok(Some(chain)) => EncodingVersion::from(chain.chain_encoding),
            Ok(None) => {
                eprintln!(
                    "Warning: supported chain {chain_key} not found while resolving block \
                     encoding; defaulting to V1"
                );
                EncodingVersion::V1
            }
            Err(e) => {
                eprintln!(
                    "Warning: failed to fetch supported chain {chain_key} encoding ({e}); \
                     defaulting to V1"
                );
                EncodingVersion::V1
            }
        },
        Err(e) => {
            eprintln!(
                "Warning: failed to connect to CC3 at {cc3_rpc_url} to resolve block encoding \
                 ({e}); defaulting to V1"
            );
            EncodingVersion::V1
        }
    }
}
