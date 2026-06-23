//! Race test (PoC test T4) — placeholder.
//!
//! Documents the contract: when two relayers race to deliver the same message, exactly one
//! should observe `MessageDelivered`; the other should hit `MessageAlreadyValidated` and
//! treat it as a successful outcome (PoC §6.5).
//!
//! Implementing this end-to-end requires:
//!  * a local Anvil node serving the inbox and the dummy outbox factory — co-located on one node
//!    for test simplicity (in production the Outbox lives on Creditcoin L1, the Inbox on the
//!    destination chain),
//!  * two relayer processes pointed at the same inbox + the same gossipsub mesh,
//!  * a fixture publisher injecting `MessagePublished` then a quorum of attester votes,
//!  * an assertion that exactly one tx mined success and the other got `AlreadyValidated`.
//!
//! That harness is gated behind the `integration-tests` feature (same as the proof-gen-api
//! crate) and is the next-iteration extension of this stub.

#![cfg(feature = "integration-tests")]

#[tokio::test]
#[ignore = "requires anvil + a fixture attester publisher; tracked under PR6 hardening"]
async fn two_relayers_one_succeeds_one_already_validated() {
    // Steps:
    // 1. Start anvil; deploy dummy Inbox + Outbox (co-located on one node for test simplicity).
    // 2. Spawn relayer A and relayer B with the same configured route + same attester set.
    // 3. Publish a MessagePublished event; gossip a quorum of votes.
    // 4. Assert: exactly one MessageDelivered receipt; the other relayer's metrics show
    //    DeliveryStatus::AlreadyValidated += 1 and zero Reverted.
    unimplemented!("see module-level docs");
}
