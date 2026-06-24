//! Off-chain acknowledgment submitter (research §05/§10).
//!
//! Trust-minimized acknowledgment is **proof-based, not vote-based**. For each route that opts in
//! (`route.ack = Some(..)`), this worker:
//!
//!  1. Watches the **destination** Inbox for `MessageDelivered(bytes32 indexed messageId)` —
//!     evidence that a message was delivered to the destination dApp.
//!  2. For the transaction that emitted it, fetches a native USC delivery proof from the proof-gen
//!     API (`GET {proof_gen_url}/api/v1/proof-by-tx/{chain_key}/{tx_hash}`): the prover `txBytes`
//!     plus the merkle-inclusion and continuity proofs.
//!  3. Submits that proof to the source-chain `AcknowledgmentValidator.submitAcknowledgment(..)`.
//!     The contract verifies the proof against the block-prover precompile, decodes the
//!     `MessageDelivered` logs, and calls `Outbox.acknowledgeMessage` for each — so the relayer
//!     never needs acknowledge authority; the proof is self-validating.
//!
//! Submission is keyed (and deduped) by destination **transaction hash**: one transaction may
//! contain several `MessageDelivered` logs and the validator acknowledges all of them in a single
//! call. A transaction whose block is not yet attested returns HTTP 422 (`BlockNotReady`) from the
//! proof-gen API and is retried on the next tick.

use std::collections::HashSet;
use std::str::FromStr;
use std::time::Duration;

use alloy::network::EthereumWallet;
use alloy::primitives::{Bytes, B256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::Filter;
use alloy::signers::local::PrivateKeySigner;
use alloy::sol_types::SolEvent;
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::abi::{ContinuityProof, IInbox, MerkleProof, MerkleProofEntry};
use crate::config::{AckConfig, ChainRoute};

/// Poll cadence for the destination `MessageDelivered` watcher and the pending-proof retry queue.
pub const ACK_POLL_INTERVAL_SECS: u64 = 6;

/// Spawn the acknowledgment submitter for one route. Returns immediately when the route has no
/// `ack` config; otherwise loops until `cancel` fires or an unrecoverable error occurs.
pub async fn run(
    route: ChainRoute,
    creditcoin_eth_rpc_url: String,
    cancel: CancellationToken,
) -> Result<()> {
    let chain_key = route.chain_key;
    let Some(ack) = route.ack.clone() else {
        debug!(chain_key, "ack disabled for route; submitter not started");
        return Ok(());
    };

    // Read-only provider on the destination chain (where MessageDelivered is emitted).
    let dest_provider = ProviderBuilder::new()
        .on_builtin(&route.destination_rpc_url)
        .await
        .with_context(|| {
            format!(
                "chain_key {chain_key}: ack submitter failed to connect to destination RPC at {}",
                route.destination_rpc_url
            )
        })?;

    // Wallet-bearing provider on the source (Creditcoin) chain, where we submit the ack.
    let signer: PrivateKeySigner = ack
        .signer_key
        .trim()
        .parse()
        .with_context(|| format!("chain_key {chain_key}: invalid ack.signer_key"))?;
    let submitter_address = signer.address();
    let source_provider = ProviderBuilder::new()
        .wallet(EthereumWallet::from(signer))
        .on_builtin(&creditcoin_eth_rpc_url)
        .await
        .with_context(|| {
            format!(
                "chain_key {chain_key}: ack submitter failed to connect to Creditcoin EVM RPC at \
                 {creditcoin_eth_rpc_url}"
            )
        })?;

    let client = ProofGenClient::new(&ack.proof_gen_url)?;

    info!(
        chain_key,
        inbox = %route.inbox_address,
        validator = %ack.validator_address,
        submitter = %submitter_address,
        proof_gen_url = %ack.proof_gen_url,
        "🧾 acknowledgment submitter online"
    );

    let mut last_seen = dest_provider.get_block_number().await.with_context(|| {
        format!("chain_key {chain_key}: ack submitter failed to read chain head")
    })?;

    // Destination tx hashes seen but not yet acknowledged (proof not ready / transient failure).
    let mut pending: HashSet<B256> = HashSet::new();
    // Tx hashes already acknowledged (or terminally skipped) — never re-submitted.
    let mut done: HashSet<B256> = HashSet::new();

    let mut tick = tokio::time::interval(Duration::from_secs(ACK_POLL_INTERVAL_SECS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                info!(chain_key, "🛑 acknowledgment submitter exiting on cancel");
                return Ok(());
            }
            _ = tick.tick() => {
                if let Err(err) = discover_delivered(
                    chain_key,
                    route.inbox_address,
                    &dest_provider,
                    &mut last_seen,
                    &mut pending,
                    &done,
                ).await {
                    warn!(chain_key, %err, "ack discovery iteration failed; will retry");
                }

                process_pending(
                    chain_key,
                    &ack,
                    &client,
                    &source_provider,
                    &mut pending,
                    &mut done,
                ).await;
            }
        }
    }
}

/// Poll the destination Inbox for new `MessageDelivered` events and enqueue their tx hashes.
async fn discover_delivered<P: Provider>(
    chain_key: u64,
    inbox: alloy::primitives::Address,
    provider: &P,
    last_seen: &mut u64,
    pending: &mut HashSet<B256>,
    done: &HashSet<B256>,
) -> Result<()> {
    let tip = provider.get_block_number().await?;
    if tip <= *last_seen {
        return Ok(());
    }
    let from_block = *last_seen + 1;

    let filter = Filter::new()
        .address(inbox)
        .event_signature(IInbox::MessageDelivered::SIGNATURE_HASH)
        .from_block(from_block)
        .to_block(tip);

    let logs = provider.get_logs(&filter).await.with_context(|| {
        format!("eth_getLogs MessageDelivered from {from_block} to {tip} failed")
    })?;

    for log in logs {
        let Some(tx_hash) = log.transaction_hash else {
            warn!(
                chain_key,
                "MessageDelivered log without transaction_hash; skipping"
            );
            continue;
        };
        if done.contains(&tx_hash) || !pending.insert(tx_hash) {
            continue;
        }
        debug!(chain_key, %tx_hash, "🧾 observed MessageDelivered; queued for acknowledgment");
    }

    *last_seen = tip;
    Ok(())
}

/// Try to fetch a proof and submit an acknowledgment for every pending destination tx. Successful
/// (or terminally-reverting) submissions move to `done`; not-yet-ready proofs stay pending.
async fn process_pending<P: Provider>(
    chain_key: u64,
    ack: &AckConfig,
    client: &ProofGenClient,
    source_provider: &P,
    pending: &mut HashSet<B256>,
    done: &mut HashSet<B256>,
) {
    let todo: Vec<B256> = pending.iter().copied().collect();
    for tx_hash in todo {
        match acknowledge_tx(chain_key, ack, client, source_provider, tx_hash).await {
            Ok(AckOutcome::Acknowledged) => {
                info!(chain_key, %tx_hash, "✅ delivery acknowledged on source Outbox");
                pending.remove(&tx_hash);
                done.insert(tx_hash);
            }
            Ok(AckOutcome::Terminal(reason)) => {
                warn!(chain_key, %tx_hash, %reason, "ack skipped (terminal); will not retry");
                pending.remove(&tx_hash);
                done.insert(tx_hash);
            }
            Ok(AckOutcome::NotReady) => {
                debug!(chain_key, %tx_hash, "proof not ready yet; will retry");
            }
            Err(err) => {
                warn!(chain_key, %tx_hash, %err, "ack attempt failed transiently; will retry");
            }
        }
    }
}

enum AckOutcome {
    /// Proof verified and `acknowledgeMessage` succeeded.
    Acknowledged,
    /// The proof block is not yet attested (`BlockNotReady`); retry later.
    NotReady,
    /// A permanent condition (e.g. on-chain revert: already acknowledged / does not require ack).
    Terminal(String),
}

/// Fetch the delivery proof for `tx_hash` and submit it to the source `AcknowledgmentValidator`.
async fn acknowledge_tx<P: Provider>(
    chain_key: u64,
    ack: &AckConfig,
    client: &ProofGenClient,
    source_provider: &P,
    tx_hash: B256,
) -> Result<AckOutcome> {
    let proof = match client.proof_by_tx(chain_key, tx_hash).await? {
        ProofFetch::Ready(p) => p,
        ProofFetch::NotReady => return Ok(AckOutcome::NotReady),
    };

    let encoded_tx = proof.encoded_transaction()?;
    let (merkle_proof, continuity_proof) = proof.to_proofs()?;
    let height = proof.header_number;

    let validator =
        crate::abi::IAcknowledgmentValidator::new(ack.validator_address, source_provider);

    let pending_tx = validator
        .submitAcknowledgment(height, encoded_tx, merkle_proof, continuity_proof)
        .send()
        .await;

    match pending_tx {
        Ok(builder) => match builder.get_receipt().await {
            Ok(receipt) if receipt.status() => Ok(AckOutcome::Acknowledged),
            Ok(_) => Ok(AckOutcome::Terminal("tx mined but reverted".into())),
            Err(err) => Err(anyhow!("receipt fetch failed: {err}")),
        },
        Err(err) if is_terminal_revert(&err) => Ok(AckOutcome::Terminal(err.to_string())),
        Err(err) => Err(anyhow!("submitAcknowledgment send failed: {err}")),
    }
}

/// Classify a submit error as a permanent on-chain revert (vs. a transient RPC failure). Already
/// acknowledged / does-not-require-ack / proof-verification reverts are permanent — retrying will
/// only revert again.
fn is_terminal_revert(err: &impl std::fmt::Display) -> bool {
    let s = err.to_string();
    s.contains("AlreadyAcknowledged")
        || s.contains("DoesNotRequireAck")
        || s.contains("MessageNotFound")
        || s.contains("ProofVerificationFailed")
        || s.contains("NoMessageDeliveredLogs")
        || s.contains("execution reverted")
}

// ---------------------------------------------------------------------------
// proof-gen API client
// ---------------------------------------------------------------------------

/// Minimal HTTP client for the proof-gen API server's `proof-by-tx` endpoint.
struct ProofGenClient {
    http: reqwest::Client,
    base: String,
}

enum ProofFetch {
    Ready(SingleContinuityResponse),
    /// HTTP 422 — the block containing the tx is not yet attested.
    NotReady,
}

impl ProofGenClient {
    fn new(base_url: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("failed to build proof-gen HTTP client")?;
        Ok(Self {
            http,
            base: base_url.trim_end_matches('/').to_string(),
        })
    }

    async fn proof_by_tx(&self, chain_key: u64, tx_hash: B256) -> Result<ProofFetch> {
        let url = format!(
            "{}/api/v1/proof-by-tx/{}/{:#x}",
            self.base, chain_key, tx_hash
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url} failed"))?;

        // 422 (BlockNotReady) is expected while the destination block is still being attested.
        if resp.status() == reqwest::StatusCode::UNPROCESSABLE_ENTITY {
            return Ok(ProofFetch::NotReady);
        }
        let status = resp.status();
        let body = resp
            .text()
            .await
            .with_context(|| format!("reading body of {url}"))?;
        if !status.is_success() {
            anyhow::bail!("proof-gen returned {status} for {url}: {body}");
        }
        let parsed: SingleContinuityResponse = serde_json::from_str(&body)
            .with_context(|| format!("decoding proof-gen response from {url}"))?;
        Ok(ProofFetch::Ready(parsed))
    }
}

// ---------------------------------------------------------------------------
// proof-gen response shape (mirrors proof-gen-api-server SingleContinuityResponse)
// ---------------------------------------------------------------------------

/// Subset of the proof-gen `SingleContinuityResponse` the submitter needs. Field names are
/// camelCase to match the server's `#[serde(rename_all = "camelCase")]`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SingleContinuityResponse {
    header_number: u64,
    /// Hex-encoded prover `txBytes` (encoded tx + receipt). `None` when the server only returned a
    /// continuity proof (no merkle inclusion) — which would not satisfy the validator.
    tx_bytes: Option<String>,
    continuity_proof: ContinuityProofJson,
    merkle_proof: MerkleProofJson,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContinuityProofJson {
    lower_endpoint_digest: String,
    roots: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct MerkleProofJson {
    root: String,
    siblings: Vec<MerkleProofEntryJson>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MerkleProofEntryJson {
    hash: String,
    is_left: bool,
}

impl SingleContinuityResponse {
    /// Hex-decode the prover `txBytes` into the calldata the validator expects.
    fn encoded_transaction(&self) -> Result<Bytes> {
        let raw = self.tx_bytes.as_deref().context(
            "proof-gen response missing txBytes (continuity-only proof cannot be acked)",
        )?;
        let bytes =
            hex::decode(raw.trim_start_matches("0x")).context("txBytes is not valid hex")?;
        Ok(Bytes::from(bytes))
    }

    /// Convert the JSON proof bundle into the `sol!`-generated argument structs.
    fn to_proofs(&self) -> Result<(MerkleProof, ContinuityProof)> {
        let merkle = MerkleProof {
            root: parse_b256(&self.merkle_proof.root).context("merkle_proof.root")?,
            siblings: self
                .merkle_proof
                .siblings
                .iter()
                .map(|s| {
                    Ok(MerkleProofEntry {
                        hash: parse_b256(&s.hash).context("merkle_proof.siblings[].hash")?,
                        isLeft: s.is_left,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        };

        let continuity = ContinuityProof {
            lowerEndpointDigest: parse_b256(&self.continuity_proof.lower_endpoint_digest)
                .context("continuity_proof.lower_endpoint_digest")?,
            roots: self
                .continuity_proof
                .roots
                .iter()
                .map(|r| parse_b256(r).context("continuity_proof.roots[]"))
                .collect::<Result<Vec<_>>>()?,
        };

        Ok((merkle, continuity))
    }
}

fn parse_b256(s: &str) -> Result<B256> {
    B256::from_str(s.trim()).with_context(|| format!("not a 32-byte hex value: {s}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "chainKey": 2,
        "headerNumber": 123,
        "txIndex": 0,
        "txHash": "0x1111111111111111111111111111111111111111111111111111111111111111",
        "txBytes": "0xdeadbeef",
        "continuityProof": {
            "lowerEndpointDigest": "0x2222222222222222222222222222222222222222222222222222222222222222",
            "roots": [
                "0x3333333333333333333333333333333333333333333333333333333333333333",
                "0x4444444444444444444444444444444444444444444444444444444444444444"
            ]
        },
        "merkleProof": {
            "root": "0x5555555555555555555555555555555555555555555555555555555555555555",
            "siblings": [
                { "hash": "0x6666666666666666666666666666666666666666666666666666666666666666", "isLeft": true },
                { "hash": "0x7777777777777777777777777777777777777777777777777777777777777777", "isLeft": false }
            ]
        },
        "cached": false,
        "generatedAt": "2026-06-24T00:00:00Z"
    }"#;

    #[test]
    fn decodes_proof_gen_response() {
        let parsed: SingleContinuityResponse = serde_json::from_str(SAMPLE).unwrap();
        assert_eq!(parsed.header_number, 123);

        let enc = parsed.encoded_transaction().unwrap();
        assert_eq!(enc.to_vec(), vec![0xde, 0xad, 0xbe, 0xef]);

        let (merkle, continuity) = parsed.to_proofs().unwrap();
        assert_eq!(merkle.siblings.len(), 2);
        assert!(merkle.siblings[0].isLeft);
        assert!(!merkle.siblings[1].isLeft);
        assert_eq!(continuity.roots.len(), 2);
    }

    #[test]
    fn missing_tx_bytes_is_error() {
        let json = SAMPLE.replace("\"txBytes\": \"0xdeadbeef\",", "");
        let parsed: SingleContinuityResponse = serde_json::from_str(&json).unwrap();
        assert!(parsed.encoded_transaction().is_err());
    }

    #[test]
    fn terminal_revert_classification() {
        assert!(is_terminal_revert(&"reverted: MessageAlreadyAcknowledged"));
        assert!(is_terminal_revert(&"execution reverted"));
        assert!(!is_terminal_revert(
            &"error sending request: connection refused"
        ));
    }
}
