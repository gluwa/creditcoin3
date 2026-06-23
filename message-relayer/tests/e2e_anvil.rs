//! End-to-end test (PoC test T2) — placeholder.
//!
//! Asserts that `MessagePublished -> attestor votes -> deliverMessage -> MessageDelivered`
//! works against a local Anvil node (hosting both the Outbox and Inbox, co-located for test
//! simplicity) and a fixture libp2p mesh. Gated behind the `integration-tests` feature like
//! proof-gen-api-server's E2E tests.
//!
//! Concrete implementation is left for a follow-up PR — the runtime modules necessary
//! (events, p2p, pool, delivery) all expose `pub fn run(...)` so the harness can compose
//! them directly without going through `bin/relayer.rs`.

#![cfg(feature = "integration-tests")]

#[tokio::test]
#[ignore = "requires alloy-node-bindings (anvil) + a fixture attestor publisher"]
async fn message_published_to_message_delivered_round_trip() {
    // Steps:
    // 1. Boot anvil via alloy-node-bindings; deploy dummy Outbox + Inbox (co-located on one node
    //    for test simplicity).
    // 2. Spawn libp2p subscriber + outbox watcher + pool + delivery worker.
    // 3. Emit MessagePublished from Outbox; gossip 2N/3+1 valid votes on
    //    `{chain_key}/message-votes/v1` from the fixture publisher.
    // 4. Wait for MessageDelivered receipt on the inbox; assert metrics counters
    //    matched the expected outcomes.
    unimplemented!("see module-level docs");
}
