//! Outbox address resolution.
//!
//! Production: a factory contract on Creditcoin L1 maps `bytes32 chainKey` → `Outbox` address
//! (PoC PDF §4). PoC: the relayer falls back to the `outbox_address` set on each [`ChainRoute`]
//! when no factory has been deployed yet. The trait is in place so swapping in the real factory
//! is a one-impl change rather than a refactor across modules.

use alloy::primitives::Address;
use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::config::ChainRoute;

/// Pluggable strategy for resolving an Outbox address for a given route.
#[async_trait]
pub trait OutboxResolver: Send + Sync + std::fmt::Debug {
    async fn resolve(&self, route: &ChainRoute) -> Result<Address>;
}

/// PoC default: take whatever the operator put in `route.outbox_address` and refuse to start
/// otherwise. When the production factory ships, replace this with [`FactoryResolver`].
#[derive(Debug, Default)]
pub struct ConfigOverrideResolver;

#[async_trait]
impl OutboxResolver for ConfigOverrideResolver {
    async fn resolve(&self, route: &ChainRoute) -> Result<Address> {
        route.outbox_address.with_context(|| {
            format!(
                "chain_key {} has no outbox_address and the factory resolver is not yet \
                 wired in this PoC build — set `outbox_address` in the route config",
                route.chain_key
            )
        })
    }
}

/// Future production resolver: calls `OutboxFactory.outboxFor(bytes32 chainKey)` on Creditcoin
/// L1 and returns the resulting address. Stubbed until the factory contract lands; the type is
/// declared so callers can wire `Arc<dyn OutboxResolver>` through without churn later.
#[derive(Debug)]
pub struct FactoryResolver {
    pub factory_address: Address,
}

#[async_trait]
impl OutboxResolver for FactoryResolver {
    async fn resolve(&self, _route: &ChainRoute) -> Result<Address> {
        anyhow::bail!(
            "FactoryResolver is a placeholder until the OutboxFactory contract is deployed; \
             configure ConfigOverrideResolver and set route.outbox_address explicitly."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AttesterSet, ChainRoute};
    use alloy::primitives::address;

    fn route_with(outbox: Option<Address>) -> ChainRoute {
        ChainRoute {
            chain_key: 2,
            creditcoin_chain_id: 1,
            outbox_address: outbox,
            destination_rpc_url: "http://x".into(),
            inbox_address: address!("0000000000000000000000000000000000000002"),
            signer_key: None,
            block_confirmation_depth: 0,
            attester_set: AttesterSet::Static(vec![address!(
                "000000000000000000000000000000000000000a"
            )]),
            threshold_override: None,
        }
    }

    #[tokio::test]
    async fn config_override_returns_set_value() {
        let r = ConfigOverrideResolver;
        let addr = address!("0000000000000000000000000000000000000099");
        let out = r.resolve(&route_with(Some(addr))).await.unwrap();
        assert_eq!(out, addr);
    }

    #[tokio::test]
    async fn config_override_fails_without_value() {
        let r = ConfigOverrideResolver;
        let err = r.resolve(&route_with(None)).await.unwrap_err();
        assert!(err.to_string().contains("outbox_address"));
    }

    #[tokio::test]
    async fn factory_stub_explains_itself() {
        let r = FactoryResolver {
            factory_address: address!("00000000000000000000000000000000000000ff"),
        };
        let err = r.resolve(&route_with(None)).await.unwrap_err();
        assert!(err.to_string().contains("placeholder"));
    }
}
