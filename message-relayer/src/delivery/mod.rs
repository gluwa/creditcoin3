//! Per-route delivery worker.
//!
//! Consumes [`DeliveryJob`]s from the vote pool and submits `Inbox.deliverMessage(...)` on the
//! destination chain. Implements the PoC §7 + §9 behaviour:
//!
//!  1. (Optional) `eth_call` simulate to catch `validateVotes` reverts before paying gas.
//!  2. Send the transaction, watching for receipt.
//!  3. Classify the outcome: success / `MessageAlreadyValidated` / `MessagePending` / other.
//!  4. On `MessagePending`, schedule `retryPendingMessage` with multiplicative backoff.
//!  5. On RPC-level failure, retry up to `delivery.max_retries` with backoff.
//!
//! The worker processes one job at a time per route — serial nonce management is the simplest
//! approach for PoC scope and matches PoC §7.2 ("optional multiple wallets for throughput, out
//! of PoC scope"). Each route runs in its own [`tokio::spawn`] so a slow destination chain
//! does not block the others.

use std::time::{Duration, Instant};

use alloy::network::EthereumWallet;
use alloy::primitives::{Address, Bytes, B256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::signers::local::PrivateKeySigner;
use anyhow::{anyhow, Context, Result};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::abi::IInbox;
use crate::config::{ChainRoute, DeliveryConfig};
use crate::prom::{DeliveryStatus, Metrics};

pub mod encode;

/// Initial retry backoff. Subsequent attempts double the wait, capped by [`MAX_BACKOFF`].
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Job dispatched by the pool when a `messageHash` clears the threshold.
#[derive(Clone, Debug)]
pub struct DeliveryJob {
    pub chain_key: u64,
    pub message_id: B256,
    pub emitter: Address,
    pub message_hash: B256,
    pub payload: Vec<u8>,
    pub votes_calldata: Vec<u8>,
    pub signer_count: usize,
    pub indexed_at: Instant,
}

/// Spawn the delivery worker for one route. Exits on `cancel` or unrecoverable channel close.
pub async fn run(
    route: ChainRoute,
    delivery_config: DeliveryConfig,
    mut job_rx: mpsc::Receiver<DeliveryJob>,
    metrics: Metrics,
    cancel: CancellationToken,
) -> Result<()> {
    let chain_key = route.chain_key;
    let signer_key = route
        .signer_key
        .clone()
        .with_context(|| format!("chain_key {chain_key}: signer_key is required to deliver"))?;
    let signer: PrivateKeySigner = signer_key
        .trim()
        .parse()
        .with_context(|| format!("chain_key {chain_key}: invalid signer_key"))?;

    let signer_address = signer.address();
    let wallet = EthereumWallet::from(signer);
    let provider = ProviderBuilder::new()
        .wallet(wallet)
        .on_builtin(&route.destination_rpc_url)
        .await
        .with_context(|| {
            format!(
                "chain_key {chain_key}: failed to connect to destination RPC at {}",
                route.destination_rpc_url
            )
        })?;

    info!(
        chain_key,
        signer = %signer_address,
        inbox = %route.inbox_address,
        "🚚 delivery worker online"
    );

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                info!(chain_key, "🛑 delivery worker exiting on cancel");
                return Ok(());
            }
            maybe = job_rx.recv() => {
                let Some(job) = maybe else {
                    info!(chain_key, "delivery channel closed; worker exiting");
                    return Ok(());
                };
                if let Err(err) = handle_job(
                    &route,
                    &delivery_config,
                    &provider,
                    &job,
                    metrics.as_ref(),
                ).await {
                    error!(chain_key, message_id = %job.message_id, %err, "❌ delivery job failed");
                }
            }
        }
    }
}

async fn handle_job<P: Provider + Clone + 'static>(
    route: &ChainRoute,
    delivery_config: &DeliveryConfig,
    provider: &P,
    job: &DeliveryJob,
    metrics: &dyn crate::prom::MetricsTrait,
) -> Result<()> {
    let inbox = IInbox::new(route.inbox_address, provider);

    if delivery_config.simulate_before_send {
        if let Err(err) = inbox
            .deliverMessage(
                job.message_id,
                job.emitter,
                Bytes::from(job.payload.clone()),
                Bytes::from(job.votes_calldata.clone()),
            )
            .call()
            .await
        {
            // The inbox reverted at simulation time. If it already accepted this message we
            // treat it as success (idempotent — PoC §6.5). Otherwise we don't burn gas.
            if revert_already_validated(&err) {
                debug!(chain_key = route.chain_key, message_id = %job.message_id,
                    "simulate detected MessageAlreadyValidated; idempotent success");
                metrics.inc_deliver_tx(route.chain_key, DeliveryStatus::AlreadyValidated);
                return Ok(());
            }
            metrics.inc_deliver_tx(route.chain_key, DeliveryStatus::Reverted);
            return Err(anyhow!("simulate(deliverMessage) reverted: {err}"));
        }
    }

    metrics.inc_deliver_tx(route.chain_key, DeliveryStatus::Submitted);
    let started = Instant::now();

    let mut backoff = INITIAL_BACKOFF;
    let mut attempts = 0u32;
    let outcome = loop {
        attempts += 1;
        let pending = inbox
            .deliverMessage(
                job.message_id,
                job.emitter,
                Bytes::from(job.payload.clone()),
                Bytes::from(job.votes_calldata.clone()),
            )
            .send()
            .await;

        match pending {
            Ok(builder) => match builder.get_receipt().await {
                Ok(receipt) => {
                    if receipt.status() {
                        break SendOutcome::Succeeded;
                    }
                    // Receipt with `status = false` means the tx mined but reverted. For PoC
                    // we don't decode the revert reason from the receipt — we surface it via
                    // metrics and stop retrying (the next message will get its own attempt).
                    break SendOutcome::Reverted;
                }
                Err(err) if attempts <= delivery_config.max_retries => {
                    warn!(
                        chain_key = route.chain_key,
                        message_id = %job.message_id,
                        attempts,
                        %err,
                        "receipt fetch failed; retrying"
                    );
                }
                Err(err) => break SendOutcome::Failed(format!("receipt: {err}")),
            },
            Err(err) if revert_already_validated(&err) => {
                // Lost the race to another relayer (PoC §6.5). Treat as success.
                break SendOutcome::AlreadyValidated;
            }
            Err(err) if revert_message_pending(&err) => {
                // The inbox accepted votes but the dApp's `receiveMessage` ran out of gas.
                // Schedule a retry via the permissionless `retryPendingMessage` path.
                break SendOutcome::Pending(err.to_string());
            }
            Err(err) if attempts <= delivery_config.max_retries => {
                warn!(
                    chain_key = route.chain_key,
                    message_id = %job.message_id,
                    attempts,
                    backoff_ms = backoff.as_millis() as u64,
                    %err,
                    "send failed; retrying"
                );
            }
            Err(err) => break SendOutcome::Failed(err.to_string()),
        }

        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    };

    match outcome {
        SendOutcome::Succeeded => {
            metrics.inc_deliver_tx(route.chain_key, DeliveryStatus::Succeeded);
            metrics.observe_time_to_deliver(started.elapsed());
            info!(
                chain_key = route.chain_key,
                message_id = %job.message_id,
                signer_count = job.signer_count,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "✅ message delivered"
            );
            Ok(())
        }
        SendOutcome::AlreadyValidated => {
            metrics.inc_deliver_tx(route.chain_key, DeliveryStatus::AlreadyValidated);
            info!(
                chain_key = route.chain_key,
                message_id = %job.message_id,
                "↩️ another relayer already delivered — idempotent success"
            );
            Ok(())
        }
        SendOutcome::Pending(err_str) => {
            metrics.inc_deliver_tx(route.chain_key, DeliveryStatus::Pending);
            warn!(
                chain_key = route.chain_key,
                message_id = %job.message_id,
                err = %err_str,
                "⚠️ message left in pending state — scheduling retryPendingMessage"
            );
            // Best-effort retry; we do not block delivery of subsequent messages on this.
            spawn_pending_retry(
                (*provider).clone(),
                *inbox.address(),
                job.message_id,
                route.chain_key,
            );
            Ok(())
        }
        SendOutcome::Reverted => {
            metrics.inc_deliver_tx(route.chain_key, DeliveryStatus::Reverted);
            error!(
                chain_key = route.chain_key,
                message_id = %job.message_id,
                "❌ tx mined but reverted; no further retries"
            );
            Ok(())
        }
        SendOutcome::Failed(err_str) => {
            metrics.inc_deliver_tx(route.chain_key, DeliveryStatus::Reverted);
            Err(anyhow!("send exhausted retries: {err_str}"))
        }
    }
}

#[derive(Debug)]
enum SendOutcome {
    Succeeded,
    AlreadyValidated,
    Pending(String),
    Reverted,
    Failed(String),
}

fn revert_already_validated(err: &impl std::fmt::Display) -> bool {
    let s = err.to_string();
    // String-matching is brittle but alloy's typed revert-data API is still in flux. Once it
    // stabilizes, switch this to decoded error-data classification (selector compare against
    // `IInbox::MessageAlreadyValidated::SELECTOR`).
    s.contains("MessageAlreadyValidated") || s.contains("AlreadyValidated")
}

fn revert_message_pending(err: &impl std::fmt::Display) -> bool {
    let s = err.to_string();
    s.contains("MessagePending") || s.contains("Pending")
}

fn spawn_pending_retry<P: Provider + 'static>(
    provider: P,
    inbox_address: Address,
    message_id: B256,
    chain_key: u64,
) {
    tokio::spawn(async move {
        // Single best-effort retry after a short delay. PoC scope; production should bound
        // total attempts and persist state across restarts.
        tokio::time::sleep(Duration::from_secs(15)).await;
        let inbox = IInbox::new(inbox_address, &provider);
        match inbox.retryPendingMessage(message_id).send().await {
            Ok(builder) => match builder.get_receipt().await {
                Ok(receipt) if receipt.status() => {
                    info!(chain_key, %message_id, "♻️ retryPendingMessage succeeded");
                }
                Ok(_) => {
                    warn!(chain_key, %message_id, "retryPendingMessage tx reverted");
                }
                Err(err) => {
                    warn!(chain_key, %message_id, %err, "retryPendingMessage receipt failed");
                }
            },
            Err(err) => {
                warn!(chain_key, %message_id, %err, "retryPendingMessage send failed");
            }
        }
    });
}
