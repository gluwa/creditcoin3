//! End-to-end test for the write-ability EVM path (confluence T2, single-node slice).
//!
//! Boots a real local Anvil node, deploys a fixture Outbox whose `MessagePublished` event matches
//! the attestor's `IOutbox` binding, then drives the **actual** attestor modules against the live
//! chain:
//!
//!   resolve Outbox (`resolver`) → emit `MessagePublished` → index it (`listener::poll_once`,
//!   real `eth_getLogs`) → recompute `messageHash` → sign (`signing`) → validate + count to quorum
//!   (`ingest` + `aggregator`).
//!
//! This covers the one gap the in-crate unit tests can't: real EVM log decoding + hash binding
//! against a live node. The libp2p gossip transport is exercised separately; here we hand the
//! signed vote straight to `ingest::validate_and_count` (the same path the p2p task calls).
//!
//! `#[ignore]`d because it needs the `anvil` binary (foundry) on PATH. Run with:
//!   cargo test -p attestor --test e2e_anvil -- --ignored

use std::collections::HashSet;
use std::time::{Duration, Instant};

use alloy::network::EthereumWallet;
use alloy::primitives::{Bytes, B256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::signers::local::PrivateKeySigner;
use alloy::sol;
use alloy_node_bindings::Anvil;
use parking_lot::Mutex;

use attestor::tasks::write_ability::aggregator::VoteAggregator;
use attestor::tasks::write_ability::MessageVoteState;
use attestor::tasks::write_ability::{config::Config, ingest, listener, resolver, signing};
use write_ability::envelope::MessageVote;
use write_ability::hash::message_hash;
use write_ability::protocol::chain_key_to_bytes32;

sol! {
    #[sol(rpc, bytecode = "0x6080604052348015600e575f80fd5b506040516101ea3803806101ea833981016040819052602b916031565b5f556047565b5f602082840312156040575f80fd5b5051919050565b610196806100545f395ff3fe608060405234801561000f575f80fd5b5060043610610034575f3560e01c806370a7453214610038578063d0363ff11461004d575b5f80fd5b61004b6100463660046100b2565b610067565b005b6100555f5481565b60405190815260200160405180910390f35b336001600160a01b0316837f5260e44fe3568d7c0337cdb5eb76bd3dbf51337ac7efb8ae484a06dfcd117abe5f85856040516100a593929190610129565b60405180910390a3505050565b5f805f604084860312156100c4575f80fd5b83359250602084013567ffffffffffffffff8111156100e1575f80fd5b8401601f810186136100f1575f80fd5b803567ffffffffffffffff811115610107575f80fd5b866020828401011115610118575f80fd5b939660209190910195509293505050565b831515815260406020820152816040820152818360608301375f818301606090810191909152601f909201601f191601019291505056fea26469706673582212202fd586955a37356b87cc645342d1c2b44103ba5e52b3ab90acf1e811ecd05d5f64736f6c634300081a0033")]
    contract TestOutbox {
        constructor(bytes32 _chainKey);
        function chainKey() external view returns (bytes32);
        function publish(bytes32 messageId, bytes calldata payload) external;
        event MessagePublished(
            bytes32 indexed messageId,
            address indexed emitterAddress,
            bool requiresAck,
            bytes payload
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the anvil binary (foundry) on PATH"]
async fn outbox_publish_indexed_signed_and_reaches_quorum() {
    // 1. Boot Anvil and build a wallet-backed provider from its first dev key.
    let anvil = Anvil::new()
        .try_spawn()
        .expect("spawn anvil — is foundry installed?");
    let signer = PrivateKeySigner::from(anvil.keys()[0].clone());
    let emitter = signer.address();
    let provider = ProviderBuilder::new()
        .wallet(EthereumWallet::from(signer))
        .on_http(anvil.endpoint_url());

    // 2. Deploy the fixture Outbox bound to our chain key.
    let chain_key: u64 = 7;
    let ck_b32 = chain_key_to_bytes32(chain_key);
    let outbox = TestOutbox::deploy(&provider, ck_b32)
        .await
        .expect("deploy TestOutbox");

    // 3. Resolve it through the real resolver (config override path); the chain key comes from config.
    let mut cfg = Config::disabled();
    cfg.outbox_address = Some(*outbox.address());
    cfg.write_ability_chain_key = Some(ck_b32);
    let resolved = resolver::resolve(&provider, chain_key, &cfg)
        .await
        .expect("resolve outbox");
    assert_eq!(resolved.address, *outbox.address());
    assert_eq!(resolved.destination_chain_key, ck_b32);

    let before = provider.get_block_number().await.unwrap();

    // 4. Emit a MessagePublished.
    let message_id = B256::from([0x11u8; 32]);
    let payload = Bytes::from_static(b"hello cross-chain");
    outbox
        .publish(message_id, payload.clone())
        .send()
        .await
        .expect("send publish")
        .get_receipt()
        .await
        .expect("publish receipt");

    // 5. Index it via the real listener poll (real eth_getLogs + decode + hash).
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let mut last_seen = before;
    listener::poll_once(&provider, &resolved, 0, &mut last_seen, &tx)
        .await
        .expect("poll_once");
    let indexed = rx
        .try_recv()
        .expect("listener indexed the MessagePublished");
    assert_eq!(indexed.message_id, message_id);
    assert_eq!(indexed.emitter, emitter);

    // The listener's hash must equal an independent recomputation (the binding attesters sign).
    let expected = message_hash(
        message_id,
        emitter,
        ck_b32,
        resolved.creditcoin_chain_id,
        &payload,
    );
    assert_eq!(indexed.message_hash, expected, "messageHash must match");

    // 6. Sign and run the full validate+count path; a single-attester set (threshold 1) reaches quorum.
    let msigner = signing::MessageSigner::from_seed(&[9u8; 32]).unwrap();
    let active_set: HashSet<_> = std::iter::once(msigner.address()).collect();
    let state = MessageVoteState {
        aggregator: Mutex::new(VoteAggregator::new(1, 1000, Duration::from_secs(60))),
        active_set,
        publish_tx: tokio::sync::mpsc::channel(8).0,
    };
    // Chain-seen (the listener just indexed it).
    state
        .aggregator
        .lock()
        .note_indexed(indexed.message_hash.0, Instant::now());

    let signature = msigner.sign(&indexed.message_hash).unwrap();
    let vote = MessageVote {
        chain_key,
        message_id: indexed.message_id.0,
        message_hash: indexed.message_hash.0,
        signer: msigner.address().into_array(),
        signature,
    };
    let decision = ingest::validate_and_count(&state, chain_key, &vote.encode_bytes());
    assert!(
        matches!(
            decision,
            ingest::Acceptance::Accept {
                reached_threshold: true,
                ..
            }
        ),
        "valid chain-seen vote from an attester should reach quorum, got {decision:?}"
    );
}
