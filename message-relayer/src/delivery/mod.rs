//! Per-route delivery worker.
//!
//! Consumes [`DeliveryJob`]s from the vote pool and submits `Inbox.deliverMessage(...)` on the
//! destination chain. Implements the PoC §7 + §9 behaviour:
//!
//!  1. (Optional) `eth_call` simulate to catch `validateVotes` reverts before paying gas.
//!     Simulation distinguishes reverts (terminal / already-validated) from transport failures
//!     (returned to the pool's bounded retry) — a mere RPC blip must not drop a message.
//!  2. Send the transaction, watching for receipt (bounded by [`RECEIPT_TIMEOUT`] so a stuck
//!     underpriced tx cannot wedge the route's serial worker).
//!  3. Classify the outcome. Note `MessagePending` is an **event on a successful tx** (the inbox
//!     validated the votes but the dApp's `receiveMessage` reverted) — it is detected from the
//!     receipt logs, not from a revert.
//!  4. On `MessagePending`, schedule bounded `retryPendingMessage` attempts (permissionless).
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
use alloy::sol_types::{SolError, SolEvent};
use anyhow::{Context, Result};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::abi::IInbox;
use crate::config::{ChainRoute, DeliveryConfig};
use crate::prom::{DeliveryStatus, Metrics};
use crate::revert::{has_selector, is_revert};

pub mod encode;

/// Initial retry backoff. Subsequent attempts double the wait, capped by [`MAX_BACKOFF`].
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Upper bound on waiting for a delivery receipt. Without it, one stuck (e.g. underpriced) tx
/// blocks the route's serial worker — and every message queued behind it — indefinitely. On
/// timeout the job returns to the pool's bounded retry; if the stuck tx mines later, the next
/// attempt's simulate detects the duplicate ("Already validated") and resolves idempotently.
const RECEIPT_TIMEOUT: Duration = Duration::from_secs(120);

/// Bounded, permissionless `retryPendingMessage` schedule after a delivery lands in the
/// `MessagePending` state (dApp callback reverted). Backoff gives the destination dApp time to
/// recover (e.g. gas market spike); anyone else may also retry, so this is best-effort.
const PENDING_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(15),
    Duration::from_secs(60),
    Duration::from_secs(240),
];

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

#[derive(Clone, Debug)]
pub struct DeliveryResult {
    pub chain_key: u64,
    pub message_hash: B256,
    pub outcome: DeliveryResultKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryResultKind {
    Delivered,
    Terminal,
    Retryable,
}

/// Spawn the delivery worker for one route. Exits on `cancel` or unrecoverable channel close.
pub async fn run(
    route: ChainRoute,
    delivery_config: DeliveryConfig,
    mut job_rx: mpsc::Receiver<DeliveryJob>,
    result_tx: mpsc::Sender<DeliveryResult>,
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
                let outcome = match handle_job(
                    &route,
                    &delivery_config,
                    &provider,
                    &job,
                    metrics.as_ref(),
                ).await {
                    Ok(outcome) => outcome,
                    Err(err) => {
                        error!(chain_key, message_id = %job.message_id, %err, "❌ delivery job failed");
                        DeliveryResultKind::Retryable
                    }
                };
                if result_tx
                    .send(DeliveryResult {
                        chain_key: job.chain_key,
                        message_hash: job.message_hash,
                        outcome,
                    })
                    .await
                    .is_err()
                {
                    warn!(chain_key, "delivery result channel closed; worker exiting");
                    return Ok(());
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
) -> Result<DeliveryResultKind> {
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
            // If the inbox already accepted this message we treat it as success (idempotent —
            // PoC §6.5). Any other *revert* is deterministic, so we don't burn gas. A transport
            // failure (RPC blip, timeout) is neither — the pool retries it with backoff; treating
            // it as terminal would silently drop a deliverable message.
            if revert_already_validated(&err) {
                debug!(chain_key = route.chain_key, message_id = %job.message_id,
                    "simulate detected already-validated; idempotent success");
                metrics.inc_deliver_tx(route.chain_key, DeliveryStatus::AlreadyValidated);
                return Ok(DeliveryResultKind::Delivered);
            }
            if is_revert(&err.to_string()) {
                metrics.inc_deliver_tx(route.chain_key, DeliveryStatus::Reverted);
                warn!(
                    chain_key = route.chain_key,
                    message_id = %job.message_id,
                    %err,
                    "simulate(deliverMessage) reverted; treating as terminal"
                );
                return Ok(DeliveryResultKind::Terminal);
            }
            warn!(
                chain_key = route.chain_key,
                message_id = %job.message_id,
                %err,
                "simulate(deliverMessage) failed at transport level; returning to pool for retry"
            );
            return Ok(DeliveryResultKind::Retryable);
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
            Ok(builder) => match tokio::time::timeout(RECEIPT_TIMEOUT, builder.get_receipt()).await
            {
                Ok(Ok(receipt)) => {
                    if receipt.status() {
                        // `deliverMessage` succeeds even when the dApp callback reverts — the
                        // inbox stores the message and emits `MessagePending` instead of
                        // `MessageDelivered`. Detect that from the receipt logs; it is NOT a
                        // revert (see SimpleInbox.deliverMessage's try/catch).
                        let left_pending = receipt.inner.logs().iter().any(|l| {
                            l.address() == route.inbox_address
                                && l.topics().first()
                                    == Some(&IInbox::MessagePending::SIGNATURE_HASH)
                        });
                        if left_pending {
                            break SendOutcome::Pending;
                        }
                        break SendOutcome::Succeeded;
                    }
                    // Receipt with `status = false` means the tx mined but reverted. For PoC
                    // we don't decode the revert reason from the receipt — we surface it via
                    // metrics and stop retrying (the next message will get its own attempt).
                    break SendOutcome::Reverted("tx mined but reverted".into());
                }
                Ok(Err(err)) if attempts <= delivery_config.max_retries => {
                    warn!(
                        chain_key = route.chain_key,
                        message_id = %job.message_id,
                        attempts,
                        %err,
                        "receipt fetch failed; retrying"
                    );
                }
                Ok(Err(err)) => break SendOutcome::Failed(format!("receipt: {err}")),
                Err(_elapsed) => {
                    // Stuck / underpriced tx: stop blocking the route. The pool retries with
                    // backoff; if this tx mines meanwhile, the next simulate resolves it as
                    // already-validated.
                    break SendOutcome::Failed(format!(
                        "no receipt within {RECEIPT_TIMEOUT:?} (tx possibly stuck)"
                    ));
                }
            },
            Err(err) if revert_already_validated(&err) => {
                // Lost the race to another relayer (PoC §6.5). Treat as success.
                break SendOutcome::AlreadyValidated;
            }
            Err(err) if is_revert(&err.to_string()) => {
                // Deterministic contract revert at send / gas-estimation time — retrying would
                // revert identically, so don't burn the retry budget on it.
                break SendOutcome::Reverted(err.to_string());
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
            Ok(DeliveryResultKind::Delivered)
        }
        SendOutcome::AlreadyValidated => {
            metrics.inc_deliver_tx(route.chain_key, DeliveryStatus::AlreadyValidated);
            info!(
                chain_key = route.chain_key,
                message_id = %job.message_id,
                "↩️ another relayer already delivered — idempotent success"
            );
            Ok(DeliveryResultKind::Delivered)
        }
        SendOutcome::Pending => {
            metrics.inc_deliver_tx(route.chain_key, DeliveryStatus::Pending);
            warn!(
                chain_key = route.chain_key,
                message_id = %job.message_id,
                "⚠️ votes validated but the dApp callback reverted — message left pending; \
                 scheduling bounded retryPendingMessage attempts"
            );
            // The votes are consumed on-chain (`validatedMessages[messageId] = true`), so from the
            // pool's perspective delivery is complete — a re-dispatch would revert as a duplicate.
            // The remaining `retryPendingMessage` work is permissionless best-effort.
            spawn_pending_retry(
                (*provider).clone(),
                *inbox.address(),
                job.message_id,
                route.chain_key,
            );
            Ok(DeliveryResultKind::Delivered)
        }
        SendOutcome::Reverted(reason) => {
            metrics.inc_deliver_tx(route.chain_key, DeliveryStatus::Reverted);
            error!(
                chain_key = route.chain_key,
                message_id = %job.message_id,
                %reason,
                "❌ delivery reverted; no further retries"
            );
            Ok(DeliveryResultKind::Terminal)
        }
        SendOutcome::Failed(err_str) => {
            metrics.inc_deliver_tx(route.chain_key, DeliveryStatus::Reverted);
            warn!(
                chain_key = route.chain_key,
                message_id = %job.message_id,
                err = %err_str,
                "send exhausted delivery worker retries; returning to pool for bounded retry"
            );
            Ok(DeliveryResultKind::Retryable)
        }
    }
}

#[derive(Debug)]
enum SendOutcome {
    Succeeded,
    AlreadyValidated,
    /// Tx succeeded but the receipt carries `MessagePending` — the dApp callback reverted and the
    /// message is stored for `retryPendingMessage`.
    Pending,
    /// Deterministic revert (mined-and-reverted, or revert at send/estimation time).
    Reverted(String),
    /// Transient infrastructure failure — returned to the pool's bounded retry.
    Failed(String),
}

/// Whether a `deliverMessage` error means the inbox already accepted this message (idempotent
/// success — we lost the race to another relayer, or a previous stuck attempt mined).
///
/// Matched three ways because node dialects differ: the deployed `SimpleInbox` rejects duplicates
/// with `require(..., "Already validated")` (a *string* revert), the custom error name covers
/// future inbox versions on nodes that decode names, and the selector covers nodes that return
/// raw revert data (see [`crate::revert`]).
fn revert_already_validated(err: &impl std::fmt::Display) -> bool {
    let s = err.to_string();
    s.contains("Already validated")
        || s.contains("MessageAlreadyValidated")
        || has_selector(&s, IInbox::MessageAlreadyValidated::SELECTOR)
}

/// Bounded, detached best-effort `retryPendingMessage` attempts. Detached because it must not
/// block the route's serial delivery worker; bounded ([`PENDING_RETRY_DELAYS`]) because the call
/// is permissionless — anyone (including a future relayer restart) can retry a message that is
/// still pending, so giving up here strands nothing.
fn spawn_pending_retry<P: Provider + 'static>(
    provider: P,
    inbox_address: Address,
    message_id: B256,
    chain_key: u64,
) {
    tokio::spawn(async move {
        let inbox = IInbox::new(inbox_address, &provider);
        for (attempt, delay) in PENDING_RETRY_DELAYS.iter().enumerate() {
            tokio::time::sleep(*delay).await;
            // Someone (a dApp user, another relayer) may have completed the retry meanwhile.
            match inbox.isPending(message_id).call().await {
                Ok(ret) if !ret._0 => {
                    info!(chain_key, %message_id, "♻️ pending message already resolved");
                    return;
                }
                Ok(_) => {}
                Err(err) => {
                    warn!(chain_key, %message_id, %err, "isPending check failed; attempting retry anyway");
                }
            }
            match inbox.retryPendingMessage(message_id).send().await {
                Ok(builder) => {
                    match tokio::time::timeout(RECEIPT_TIMEOUT, builder.get_receipt()).await {
                        Ok(Ok(receipt)) if receipt.status() => {
                            info!(chain_key, %message_id, "♻️ retryPendingMessage succeeded");
                            return;
                        }
                        Ok(Ok(_)) => {
                            warn!(chain_key, %message_id, attempt, "retryPendingMessage tx reverted");
                        }
                        Ok(Err(err)) => {
                            warn!(chain_key, %message_id, attempt, %err, "retryPendingMessage receipt failed");
                        }
                        Err(_) => {
                            warn!(chain_key, %message_id, attempt, "retryPendingMessage receipt timed out");
                        }
                    }
                }
                Err(err) => {
                    warn!(chain_key, %message_id, attempt, %err, "retryPendingMessage send failed");
                }
            }
        }
        warn!(
            chain_key,
            %message_id,
            "retryPendingMessage attempts exhausted; message stays retryable on-chain \
             (permissionless retryPendingMessage)"
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn already_validated_matches_all_dialects() {
        // The deployed SimpleInbox string revert.
        assert!(revert_already_validated(
            &"execution reverted: Already validated"
        ));
        // Decoded custom-error name (future inbox versions).
        assert!(revert_already_validated(
            &"reverted: MessageAlreadyValidated"
        ));
        // Raw selector data (Creditcoin-style node).
        let sel = alloy::hex::encode(IInbox::MessageAlreadyValidated::SELECTOR);
        assert!(revert_already_validated(&format!(
            "VM Exception while processing transaction: revert, data: \"0x{sel}\""
        )));
        // A transport failure is not a duplicate.
        assert!(!revert_already_validated(&"connection refused"));
        // An unrelated revert is not a duplicate either.
        assert!(!revert_already_validated(
            &"execution reverted: VotesBelowThreshold"
        ));
    }
}
