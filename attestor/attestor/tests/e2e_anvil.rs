//! End-to-end test for the write-ability EVM path (confluence T2, single-node slice).
//!
//! Boots a real local Anvil node standing in for the **Creditcoin L1 EVM** — the chain the Outbox
//! and its factory live on — deploys a fixture Outbox there whose `MessagePublished` event matches
//! the attestor's `IOutbox` binding, then drives the **actual** attestor modules against the live
//! chain:
//!
//!   resolve Outbox (`resolver`) → emit `MessagePublished` → index it (`listener::poll_once`,
//!   real `eth_getLogs`) → recompute `messageHash` → sign (`signing`) → validate + count to quorum
//!   (`ingest` + `aggregator`).
//!
//! The attestor only ever watches the Outbox on the Creditcoin L1 EVM; it never interacts with the
//! destination chain or the Inbox (those live on the relayer's delivery path). So the Anvil node
//! here represents the Creditcoin L1 EVM, not a destination chain.
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
use alloy::providers::{ext::AnvilApi, Provider, ProviderBuilder};
use alloy::signers::local::PrivateKeySigner;
use alloy::sol;
use alloy_node_bindings::Anvil;
use parking_lot::Mutex;

use attestor::tasks::write_ability::aggregator::VoteAggregator;
use attestor::tasks::write_ability::MessageVoteState;
use attestor::tasks::write_ability::{ingest, listener, reobservation, resolver, signing};
use write_ability::envelope::MessageVote;
use write_ability::hash::message_hash;
use write_ability::protocol::chain_key_to_bytes32;

sol! {
    #[sol(rpc, bytecode = "0x6080604052348015600e575f80fd5b506040516103643803806103648339818101604052810190602e9190606b565b805f81905550506091565b5f80fd5b5f819050919050565b604d81603d565b81146056575f80fd5b50565b5f815190506065816046565b92915050565b5f60208284031215607d57607c6039565b5b5f6088848285016059565b91505092915050565b6102c68061009e5f395ff3fe608060405234801561000f575f80fd5b5060043610610034575f3560e01c806370a7453214610038578063d0363ff114610054575b5f80fd5b610052600480360381019061004d9190610167565b610072565b005b61005c6100c6565b60405161006991906101d3565b60405180910390f35b3360601b6bffffffffffffffffffffffff1916837fa6e8e64f148094d0fed92fed35afd7cd97a57c879bec937f42d5c415a509ed9b5f85856040516100b993929190610260565b60405180910390a3505050565b5f5481565b5f80fd5b5f80fd5b5f819050919050565b6100e5816100d3565b81146100ef575f80fd5b50565b5f81359050610100816100dc565b92915050565b5f80fd5b5f80fd5b5f80fd5b5f8083601f84011261012757610126610106565b5b8235905067ffffffffffffffff8111156101445761014361010a565b5b6020830191508360018202830111156101605761015f61010e565b5b9250929050565b5f805f6040848603121561017e5761017d6100cb565b5b5f61018b868287016100f2565b935050602084013567ffffffffffffffff8111156101ac576101ab6100cf565b5b6101b886828701610112565b92509250509250925092565b6101cd816100d3565b82525050565b5f6020820190506101e65f8301846101c4565b92915050565b5f8115159050919050565b610200816101ec565b82525050565b5f82825260208201905092915050565b828183375f83830152505050565b5f601f19601f8301169050919050565b5f61023f8385610206565b935061024c838584610216565b61025583610224565b840190509392505050565b5f6040820190506102735f8301866101f7565b8181036020830152610286818486610234565b905094935050505056fea2646970667358221220de7af322b33ca6c27cfc178896c6aafc20ce4b46487b8532e3c15004b397fcbc64736f6c634300081a0033")]
    contract TestOutbox {
        constructor(bytes32 _chainKey);
        function chainKey() external view returns (bytes32);
        function publish(bytes32 messageId, bytes calldata payload) external;
        event MessagePublished(
            bytes32 indexed messageId,
            bytes32 indexed emitterAddress,
            bool canAck,
            bytes payload
        );
    }
    // Generated from fixtures/test-discovery.sol with solc --bin --optimize.
    #[sol(rpc, bytecode = "0x6080604052348015600e575f5ffd5b5061022d8061001c5f395ff3fe608060405234801561000f575f5ffd5b506004361061004a575f3560e01c80632ce962cf1461004e5780634ea7132714610088578063749d25ec146100bf578063d5b4c052146100f4575b5f5ffd5b61008661005c36600461013c565b6001600160a01b03919091165f908152602081905260409020805460ff1916911515919091179055565b005b6100aa610096366004610175565b5f6020819052908152604090205460ff1681565b60405190151581526020015b60405180910390f35b6100d56100cd366004610195565b503090600190565b604080516001600160a01b0390931683529015156020830152016100b6565b6100aa6101023660046101bc565b6001600160a01b03165f9081526020819052604090205460ff16919050565b80356001600160a01b0381168114610137575f5ffd5b919050565b5f5f6040838503121561014d575f5ffd5b61015683610121565b91506020830135801515811461016a575f5ffd5b809150509250929050565b5f60208284031215610185575f5ffd5b61018e82610121565b9392505050565b5f602082840312156101a5575f5ffd5b813567ffffffffffffffff8116811461018e575f5ffd5b5f5f604083850312156101cd575f5ffd5b823563ffffffff811681146101e0575f5ffd5b91506101ee60208401610121565b9050925092905056fea264697066735822122015f40316f95575ec81a9ebdf287bef6fa9cc8aa204afbc945c16e4843166b0aa64736f6c63430008250033")]
    contract TestDiscovery {
        function setActive(address outbox, bool yes) external;
    }

}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the anvil binary (foundry) on PATH"]
async fn outbox_publish_indexed_signed_and_reaches_quorum() {
    // 1. Boot Anvil (the Creditcoin L1 EVM in this test) and build a wallet-backed provider from
    //    its first dev key.
    let anvil = Anvil::new()
        .try_spawn()
        .expect("spawn anvil — is foundry installed?");
    let signer = PrivateKeySigner::from(anvil.keys()[0].clone());
    let emitter = signer.address();
    let provider = ProviderBuilder::new()
        .wallet(EthereumWallet::from(signer))
        .on_http(anvil.endpoint_url());

    // 2. Deploy the fixture Outbox on the Creditcoin L1 EVM, bound to our chain key.
    let chain_key: u64 = 7;
    let ck_b32 = chain_key_to_bytes32(chain_key);
    let outbox = TestOutbox::deploy(&provider, ck_b32)
        .await
        .expect("deploy TestOutbox");

    // 3. Install a test Discovery/chain-info fixture at the precompile address. The fixture
    // returns itself as Discovery and authenticates the deployed Outbox using historical storage.
    let discovery = TestDiscovery::deploy(&provider).await.unwrap();
    let code = provider.get_code_at(*discovery.address()).await.unwrap();
    provider
        .anvil_set_code(resolver::CHAIN_INFO_PRECOMPILE, code)
        .await
        .unwrap();
    let registry = TestDiscovery::new(resolver::CHAIN_INFO_PRECOMPILE, &provider);
    registry
        .setActive(*outbox.address(), true)
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    let resolved = resolver::resolve(&provider, chain_key, ck_b32)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved.destination_chain_key, ck_b32);

    let before = provider.get_block_number().await.unwrap();

    // 4. Emit a MessagePublished.
    let message_id = B256::from([0x11u8; 32]);
    let payload = Bytes::from_static(b"hello cross-chain");
    let published = outbox
        .publish(message_id, payload.clone())
        .send()
        .await
        .expect("send publish")
        .get_receipt()
        .await
        .expect("publish receipt");

    // Remove the Outbox before the listener catches up. Historical membership must still admit
    // its final valid message, even though today's Discovery state no longer authorizes it.
    registry
        .setActive(*outbox.address(), false)
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    // 5. Index it via the real listener poll (real eth_getLogs + historical eth_call + hash).
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let mut last_seen = before;
    // Anvil has no GRANDPA finality, so drive the listener with the deterministic depth policy
    // (depth 0 = index up to tip) rather than the finalized-head policy used in production.
    let mut finality = listener::FinalityTracker::new(std::time::Instant::now());
    listener::poll_once(
        &provider,
        &resolved,
        &listener::FinalityPolicy::Depth(0),
        &mut finality,
        &mut last_seen,
        &tx,
    )
    .await
    .expect("poll_once");
    let indexed = rx
        .try_recv()
        .expect("listener indexed the MessagePublished");
    assert_eq!(indexed.message_id, message_id);
    assert_eq!(indexed.emitter, emitter);

    // The listener's hash must equal an independent recomputation (the binding attestors sign).
    let expected = message_hash(
        message_id,
        emitter,
        *outbox.address(),
        ck_b32,
        resolved.creditcoin_chain_id,
        &payload,
    );
    assert_eq!(indexed.message_hash, expected, "messageHash must match");
    // Anvil models its finalized tag with a block lag. Advance it past the publication so the
    // reobservation path exercises lifecycle authorization after its independent finality gate.
    provider.anvil_mine(Some(64), None).await.unwrap();
    let recovered = reobservation::reobserve(
        &provider,
        &resolved,
        3,
        &write_ability::envelope::ReobservationRequest {
            chain_key,
            message_id: message_id.0,
            tx_hash: published.transaction_hash.0,
            block_height: published.block_number.unwrap(),
        },
    )
    .await
    .unwrap()
    .expect("removed Outbox's historical message must remain recoverable");
    assert_eq!(recovered.message_hash, expected);

    // 6. Sign and run the full validate+count path; a single-attestor set (threshold 1) reaches quorum.
    let msigner = signing::MessageSigner::from_seed(&[9u8; 32]).unwrap();
    let active_set: HashSet<_> = std::iter::once(msigner.address()).collect();
    let state = MessageVoteState {
        aggregator: Mutex::new(VoteAggregator::new(1, 1000, Duration::from_secs(60))),
        active_set: parking_lot::RwLock::new(active_set),
        publish_tx: tokio::sync::mpsc::channel(8).0,
        set_update_publish_tx: tokio::sync::mpsc::channel(8).0,
        reobs_tx: tokio::sync::mpsc::channel(8).0,
        destination_chain_key: parking_lot::RwLock::new(Some(ck_b32)),
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
        "valid chain-seen vote from an attestor should reach quorum, got {decision:?}"
    );
}
