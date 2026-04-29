//! Abuse / DoS resistance test (PoC test T3).
//!
//! Spawns the real pool task and floods it with junk gossip:
//!
//!  * votes for `messageHash`es never indexed (chain-first allowlist drop),
//!  * votes by signers not in the attester allowlist,
//!  * grossly more messages than `vote_cache.max_messages` permits.
//!
//! Asserts no [`DeliveryJob`] is emitted and the pool process keeps running.

use std::collections::HashMap;
use std::time::Duration;

use alloy::primitives::address;
use message_relayer::config::VoteCacheConfig;
use message_relayer::events::IndexedMessage;
use message_relayer::p2p::MessageVote;
use message_relayer::pool::{run as run_pool, PoolHandles, RouteAttesters};
use message_relayer::prom::NoopMetrics;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn pool_drops_unknown_messages_and_emits_no_jobs() {
    let (_indexed_tx, indexed_rx) = mpsc::channel::<IndexedMessage>(16);
    let (vote_tx, vote_rx) = mpsc::channel::<MessageVote>(8192);
    let (delivery_tx, mut delivery_rx) = mpsc::channel(16);

    let route = RouteAttesters {
        chain_key: 2,
        attesters: vec![address!("000000000000000000000000000000000000000a")],
        threshold: 1,
    };

    let mut delivery_txs = HashMap::new();
    delivery_txs.insert(2u64, delivery_tx);

    let cache = VoteCacheConfig {
        ttl_seconds: 60,
        max_messages: 100,
    };
    let cancel = CancellationToken::new();
    let cancel_for_pool = cancel.clone();

    let handle = tokio::spawn(run_pool(
        vec![route],
        cache,
        PoolHandles {
            indexed_rx,
            vote_rx,
            delivery_txs,
        },
        NoopMetrics::new(),
        cancel_for_pool,
    ));

    // 1 000 votes for unique never-indexed message hashes — chain-first allowlist must drop
    // each one without growing pool state.
    for i in 0u32..1_000 {
        let mut h = [0u8; 32];
        h[..4].copy_from_slice(&i.to_le_bytes());
        let vote = MessageVote {
            chain_key: 2,
            message_id: [0u8; 32],
            message_hash: h,
            signer: [0x0au8; 20],
            signature: [0u8; 65],
        };
        let _ = vote_tx.send(vote).await;
    }

    // Give the pool task time to drain.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // No DeliveryJob should ever be produced.
    match tokio::time::timeout(Duration::from_millis(50), delivery_rx.recv()).await {
        Ok(Some(job)) => panic!("unexpected delivery job: {job:?}"),
        Ok(None) | Err(_) => {}
    }

    cancel.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}

#[tokio::test]
async fn pool_drops_votes_from_unknown_signers() {
    let (indexed_tx, indexed_rx) = mpsc::channel::<IndexedMessage>(16);
    let (vote_tx, vote_rx) = mpsc::channel::<MessageVote>(64);
    let (delivery_tx, mut delivery_rx) = mpsc::channel(16);

    let allowed = address!("000000000000000000000000000000000000000a");
    let route = RouteAttesters {
        chain_key: 2,
        attesters: vec![allowed],
        threshold: 1,
    };
    let mut delivery_txs = HashMap::new();
    delivery_txs.insert(2u64, delivery_tx);

    let cancel = CancellationToken::new();
    let cancel_for_pool = cancel.clone();
    let handle = tokio::spawn(run_pool(
        vec![route],
        VoteCacheConfig {
            ttl_seconds: 60,
            max_messages: 100,
        },
        PoolHandles {
            indexed_rx,
            vote_rx,
            delivery_txs,
        },
        NoopMetrics::new(),
        cancel_for_pool,
    ));

    // Index one message so its messageHash is in the allowlist, then send votes signed by an
    // unknown signer — they must all be rejected.
    let hash = [9u8; 32];
    let indexed = IndexedMessage {
        chain_key: 2,
        message_id: alloy::primitives::B256::from([7u8; 32]),
        emitter: address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        destination_chain_key: alloy::primitives::B256::ZERO,
        creditcoin_chain_id: 1,
        payload: vec![],
        message_hash: alloy::primitives::B256::from(hash),
    };
    let _ = indexed_tx.send(indexed).await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    for i in 0u8..32 {
        let mut signer = [0u8; 20];
        signer[19] = i;
        let vote = MessageVote {
            chain_key: 2,
            message_id: [7u8; 32],
            message_hash: hash,
            signer, // not in the allowlist
            signature: [0u8; 65],
        };
        let _ = vote_tx.send(vote).await;
    }

    match tokio::time::timeout(Duration::from_millis(150), delivery_rx.recv()).await {
        Ok(Some(job)) => panic!("unexpected delivery from unknown signers: {job:?}"),
        Ok(None) | Err(_) => {}
    }

    cancel.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;
}
