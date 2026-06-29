//! Per-route on-chain attestor-set watcher.
//!
//! For routes whose attestor set is sourced on-chain (`AttestorSet::OnChain { Evm }`), this worker
//! periodically reads the destination validator's `attestors()` + `threshold()` and pushes a
//! [`RouteAttestors`] update to the vote pool whenever they change. That lets an operator
//! add/remove an attestor on the `EOAValidator` (or change the threshold) and have the relayer pick
//! it up **without a restart** — closing the "static set, resolved once" gap. Routes with a static
//! configured set don't run a watcher.

use std::time::Duration;

use alloy::primitives::Address;
use alloy::providers::{Provider, ProviderBuilder};
use anyhow::{Context, Result};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::abi::IVoteValidator;
use crate::config::{AttestorSet, AttestorSource, ChainRoute};
use crate::pool::RouteAttestors;

/// How often to re-read the on-chain attestor set. Set changes are rare, so a slow poll keeps RPC
/// load negligible while still bounding how long the relayer runs on a stale set.
const ATTESTOR_SET_POLL_SECS: u64 = 30;

/// Run the attestor-set watcher for one route. Only `AttestorSet::OnChain { Evm }` routes should be
/// spawned (callers filter); the `Cc3` source is not yet implemented and a static set never changes.
pub async fn run(
    route: ChainRoute,
    set_tx: mpsc::Sender<RouteAttestors>,
    cancel: CancellationToken,
) -> Result<()> {
    let chain_key = route.chain_key;
    let validator = match &route.attestor_set {
        AttestorSet::OnChain {
            source: AttestorSource::Evm { address },
        } => *address,
        AttestorSet::OnChain {
            source: AttestorSource::Cc3 { .. },
        } => {
            warn!(
                chain_key,
                "cc3 attestor-set source is not implemented; set will not hot-reload"
            );
            return Ok(());
        }
        AttestorSet::Static(_) => return Ok(()),
    };

    let provider = ProviderBuilder::new()
        .on_builtin(&route.destination_rpc_url)
        .await
        .with_context(|| {
            format!(
                "chain_key {chain_key}: attestor-set watcher failed to connect to destination RPC at {}",
                route.destination_rpc_url
            )
        })?;

    info!(chain_key, %validator, "🛂 attestor-set watcher online");

    // Last (set, threshold) we pushed to the pool; `None` until the first successful read.
    let mut last: Option<(Vec<Address>, usize)> = None;

    // `interval` fires immediately on the first tick, so the set is resolved promptly at startup.
    let mut tick = tokio::time::interval(Duration::from_secs(ATTESTOR_SET_POLL_SECS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                info!(chain_key, "🛑 attestor-set watcher exiting on cancel");
                return Ok(());
            }
            _ = tick.tick() => {
                match read_set(&provider, validator).await {
                    Ok((attestors, threshold)) => {
                        let changed = last
                            .as_ref()
                            .is_none_or(|(a, t)| a != &attestors || *t != threshold);
                        if !changed {
                            continue;
                        }
                        info!(
                            chain_key,
                            attestors = attestors.len(),
                            threshold,
                            "📥 on-chain attestor set resolved/changed"
                        );
                        last = Some((attestors.clone(), threshold));
                        let update = RouteAttestors { chain_key, attestors, threshold };
                        tokio::select! {
                            res = set_tx.send(update) => {
                                if res.is_err() {
                                    warn!(chain_key, "pool set-update channel closed; watcher exiting");
                                    return Ok(());
                                }
                            }
                            () = cancel.cancelled() => return Ok(()),
                        }
                    }
                    Err(err) => {
                        warn!(chain_key, %err, "failed to read on-chain attestor set; will retry");
                    }
                }
            }
        }
    }
}

/// Read `attestors()` + `threshold()` from the destination validator contract.
async fn read_set<P: Provider>(provider: &P, validator: Address) -> Result<(Vec<Address>, usize)> {
    let validator = IVoteValidator::new(validator, provider);

    let attestors = validator
        .attestors()
        .call()
        .await
        .context("IVoteValidator.attestors() call failed")?
        ._0;

    let threshold_u256 = validator
        .threshold()
        .call()
        .await
        .context("IVoteValidator.threshold() call failed")?
        ._0;
    // Saturate rather than panic on an absurd on-chain value; an over-large threshold simply means
    // "never deliver", which is the safe direction.
    let threshold = u64::try_from(threshold_u256).unwrap_or(u64::MAX) as usize;

    Ok((attestors, threshold))
}
