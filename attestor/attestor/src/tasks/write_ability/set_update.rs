//! Attestor-set-update proposer (write-ability P2-8, off-chain half).
//!
//! Every poll, this compares the destination `EOAValidator`'s current attestor set against the
//! *elected* attestors that have registered a write-ability EVM address (the on-chain registry from
//! the `set_attestor_evm_address` extrinsic). When they differ, the attestor signs the canonical
//! update digest and gossips a [`SetUpdateVote`] on
//! [`attestor_set_update_topic`](write_ability::protocol::attestor_set_update_topic).
//!
//! Attestors only **propose** (sign + gossip) — they do not submit on-chain. The relayer snoops the
//! topic, aggregates a threshold of signatures, and calls `submitAttestorSetUpdate` (it already owns
//! destination-chain delivery, gas, and nonce management). This mirrors the message-delivery flow
//! exactly: attestors sign + gossip, relayer aggregates + delivers.
//!
//! Determinism is the crux: every attestor must sign over the **byte-identical** `newAttestors`
//! array and nonce, or the relayer can't aggregate their signatures. We enforce a canonical
//! (ascending) address ordering ([`canonical_attestor_order`]) and sign over the validator's current
//! `attestorSetUpdateNonce` — if an update lands, the nonce bumps and stale in-flight votes are
//! naturally rejected on-chain (the replay/rollback protection working as intended).

use std::sync::Arc;
use std::time::Duration;

use alloy::primitives::{Address, U256};
use alloy::providers::{Provider, ProviderBuilder};
use anyhow::{Context, Result};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use write_ability::abi::IVoteValidator;
use write_ability::envelope::SetUpdateVote;
use write_ability::hash::{attestor_set_update_digest, canonical_attestor_order};

use super::signing::MessageSigner;

/// How often to re-check whether the elected set diverged from the on-chain validator set. Set
/// changes only happen on epoch rotation, so a slow poll keeps RPC load negligible while bounding
/// how long a diverged set goes un-proposed.
const SET_UPDATE_POLL_SECS: u64 = 60;

/// Whether the destination validator's `current` set differs from our `candidate` set, compared as
/// **sets** (order- and duplicate-insensitive) — the trigger to gossip an update. `candidate` is
/// expected pre-canonicalized; the set comparison makes the decision robust regardless.
#[must_use]
pub fn set_needs_update(current: &[Address], candidate: &[Address]) -> bool {
    use std::collections::BTreeSet;
    let current: BTreeSet<&Address> = current.iter().collect();
    let candidate: BTreeSet<&Address> = candidate.iter().collect();
    current != candidate
}

/// Build a signed [`SetUpdateVote`]. `new_attestors` is canonicalized (sorted, de-duped) before
/// hashing so every attestor signs identical bytes; `chain_id` is the destination `block.chainid`
/// and `nonce` the validator's current `attestorSetUpdateNonce`.
pub fn build_set_update_vote(
    signer: &MessageSigner,
    chain_key: u64,
    new_attestors: &[Address],
    chain_id: U256,
    nonce: U256,
) -> Result<SetUpdateVote> {
    let canonical = canonical_attestor_order(new_attestors);
    let digest = attestor_set_update_digest(&canonical, chain_id, nonce);
    let signature = signer
        .sign(&digest)
        .context("sign attestor-set-update digest")?;
    Ok(SetUpdateVote {
        chain_key,
        new_attestors: canonical.iter().map(|a| a.into_array()).collect(),
        nonce: nonce.to_be_bytes::<32>(),
        signer: signer.address().into_array(),
        signature,
    })
}

/// One proposal cycle: read the elected set's EVM addresses (registry) and the validator's current
/// set/nonce/chain-id; if they diverge, return a signed vote to gossip, else `Ok(None)`.
async fn propose_once(
    cc3: &cc_client::Client,
    chain_key: u64,
    dest_rpc_url: &str,
    validator: Address,
    signer: &MessageSigner,
) -> Result<Option<SetUpdateVote>> {
    // Candidate = EVM addresses of the currently-elected attestors that have registered one.
    let candidate: Vec<Address> = cc3
        .active_attestor_evm_addresses(chain_key)
        .await
        .context("read elected attestors' EVM addresses")?
        .into_iter()
        .map(|h| Address::from_slice(h.as_bytes()))
        .collect();
    let candidate = canonical_attestor_order(&candidate);
    if candidate.is_empty() {
        // No elected attestor has registered an EVM address yet — nothing to propose. (Proposing an
        // empty set would brick the validator.)
        return Ok(None);
    }

    let provider = ProviderBuilder::new()
        .on_builtin(dest_rpc_url)
        .await
        .map_err(|e| anyhow::anyhow!("connect destination chain RPC: {e}"))?;
    let contract = IVoteValidator::new(validator, &provider);

    let current = contract
        .attestors()
        .call()
        .await
        .context("read EOAValidator.attestors()")?
        ._0;
    if !set_needs_update(&current, &candidate) {
        return Ok(None);
    }

    let nonce = contract
        .attestorSetUpdateNonce()
        .call()
        .await
        .context("read EOAValidator.attestorSetUpdateNonce()")?
        ._0;
    let chain_id = U256::from(
        provider
            .get_chain_id()
            .await
            .context("read destination chain id")?,
    );

    let vote = build_set_update_vote(signer, chain_key, &candidate, chain_id, nonce)?;
    tracing::info!(
        proposed = candidate.len(),
        current = current.len(),
        nonce = %nonce,
        "🗳️ proposing attestor-set update — signing and gossiping"
    );
    Ok(Some(vote))
}

/// Watch for elected-vs-validator set divergence and gossip a signed update vote when it appears,
/// until `token` fires. Best-effort: read/connection failures are logged and retried next tick.
/// Connects to the destination chain per-poll for the same self-healing reason as
/// [`super::attestor_set::watch`].
#[allow(clippy::too_many_arguments)]
pub async fn run_proposer(
    cc3: Arc<cc_client::Client>,
    chain_key: u64,
    dest_rpc_url: String,
    validator: Address,
    signer: MessageSigner,
    publish_tx: mpsc::Sender<SetUpdateVote>,
    token: CancellationToken,
) {
    tracing::info!(%validator, "🗳️ attestor-set-update proposer online");
    let mut tick = tokio::time::interval(Duration::from_secs(SET_UPDATE_POLL_SECS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            () = token.cancelled() => {
                tracing::info!("🛑 attestor-set-update proposer exiting on cancel");
                return;
            }
            _ = tick.tick() => {
                // Ensure this attestor's own EVM address is registered on-chain, every cycle. The
                // startup registration in `lib.rs` is best-effort and one-shot; re-checking here
                // recovers not only from a transient startup RPC blip (review P2-8 #2) but also from
                // the mapping later disappearing while the process runs — e.g. a chain-removal purge
                // (bugbot: don't latch on first success). Idempotent and cheap: `register_evm_address`
                // reads the on-chain value first and only submits an extrinsic when it is missing or
                // stale, so a steady state is a single storage read per cycle.
                super::register_evm_address(&cc3, &signer, chain_key).await;

                match propose_once(&cc3, chain_key, &dest_rpc_url, validator, &signer).await {
                    Ok(Some(vote)) => {
                        // Bounded `try_send`: if the publish channel is full the vote is dropped and
                        // re-proposed next tick (set changes persist until an update lands), so a
                        // backed-up publisher never blocks this loop.
                        if publish_tx.try_send(vote).is_err() {
                            tracing::warn!("set-update vote publish channel full — will re-propose next tick");
                        }
                    }
                    Ok(None) => {}
                    Err(err) => {
                        tracing::warn!(%err, "attestor-set-update proposal cycle failed; will retry");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::signing::recover_signer;
    use super::*;
    use alloy::primitives::address;

    #[test]
    fn set_needs_update_is_order_and_dup_insensitive() {
        let a = address!("00000000000000000000000000000000000000aa");
        let b = address!("00000000000000000000000000000000000000bb");
        // Same members, different order / dups → no update needed.
        assert!(!set_needs_update(&[a, b], &[b, a]));
        assert!(!set_needs_update(&[a, b, a], &[b, a]));
        // Added member → update needed.
        let c = address!("00000000000000000000000000000000000000cc");
        assert!(set_needs_update(&[a, b], &[a, b, c]));
        // Removed member → update needed.
        assert!(set_needs_update(&[a, b, c], &[a, b]));
        // Empty vs non-empty.
        assert!(set_needs_update(&[], &[a]));
    }

    #[test]
    fn build_vote_signs_canonical_digest_and_recovers_to_signer() {
        let signer = MessageSigner::from_seed(&[9u8; 32]).unwrap();
        let a = address!("00000000000000000000000000000000000000cc");
        let b = address!("00000000000000000000000000000000000000aa");
        let chain_id = U256::from(11_155_111u64);
        let nonce = U256::from(3u64);

        let vote = build_set_update_vote(&signer, 2, &[a, b], chain_id, nonce).unwrap();

        // Addresses are stored canonically (sorted), not in input order.
        let canonical = canonical_attestor_order(&[a, b]);
        let expected: Vec<[u8; 20]> = canonical.iter().map(|a| a.into_array()).collect();
        assert_eq!(vote.new_attestors, expected);
        assert_eq!(vote.signer, signer.address().into_array());
        assert_eq!(vote.nonce, nonce.to_be_bytes::<32>());

        // The signature recovers to our address over the digest the relayer will reconstruct.
        let digest = attestor_set_update_digest(&canonical, chain_id, nonce);
        let recovered = recover_signer(&digest, &vote.signature).unwrap();
        assert_eq!(recovered, signer.address());
    }
}
