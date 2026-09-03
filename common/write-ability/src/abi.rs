//! Solidity ABI bindings for the USC write-ability contracts, as consumed by the ATTESTOR.
//!
//! The message-relayer was extracted to gluwa/usc-message-relayer and keeps its own (wider)
//! mirror of these bindings; the cross-repo contract is that both decode the *same*
//! `MessagePublished` signature and recompute the *same* `messageHash` (see `hash.rs` and its
//! golden vectors) — a mismatch would make every signature verify as invalid on-chain. This copy
//! deliberately binds only what the attestor calls; destination-chain surfaces (`IInbox`,
//! `IAcknowledgmentValidator`, the proof envelopes) and Outbox view/error bindings the attestor
//! never reads live in the relayer repo only.
//!
//! Inline `alloy::sol!` declarations are used while the production contracts are finalized — when
//! they ship, switch each block to the JSON form (`#[sol(rpc)] interface X, "contracts/x.json"`)
//! following the pattern in `common/eth/src/evm/block_prover.rs`. Keep the function & event
//! signatures byte-identical with the production artefacts.

use alloy::sol;

sol! {
    #[sol(rpc)]
    #[derive(Debug)]
    contract IOutbox {
        /// A new cross-chain message has been published to this outbox.
        ///
        /// `messageId` is the unique handle attestors and the inbox use to track delivery.
        /// `emitterAddress` is the dApp that called `publishMessage`, emitted as `bytes32` for
        /// cross-chain consistency — the 20-byte EVM address occupies the **high** bytes
        /// (`bytes32(bytes20(emitter))`), so recover it with `Address::from_slice(&value[..20])`.
        /// `canAck` flags whether the message may be acknowledged on-chain (usc-contracts #23
        /// renamed it from `requiresAck`: the ack is optional, requested by a nonzero
        /// acknowledgmentPrice in the relayer quote) before it is
        /// considered complete. `payload` is the opaque bytes the inbox will hand to the
        /// destination dApp's `receiveMessage`.
        event MessagePublished(
            bytes32 indexed messageId,
            bytes32 indexed emitterAddress,
            bool canAck,
            bytes payload
        );

    }

    #[sol(rpc)]
    #[derive(Debug)]
    contract IOutboxFactory {
        /// Emitted by `deployOutbox` when the factory CREATE2-deploys an Outbox. The synced factory
        /// has no `getOutbox` registry — attestors discover the Outbox for their chain by scanning
        /// this event filtered on the indexed `chainKey`. `outbox` and `chainKey` are indexed
        /// (topics[1] and topics[2]).
        event OutboxCreated(
            address indexed outbox,
            uint32 indexed chainKey,
            address indexed owner,
            address validator,
            string version
        );
    }

    #[sol(rpc)]
    #[derive(Debug)]
    contract IChainInfo {
        /// `chain-info` precompile accessor (PR #873) exposing the per-chain Outbox factory
        /// address registered in `SupportedChains::OutboxFactories`. `exists` is false when no
        /// factory has been set for `chainKey`. Precompile address: `0x…0fD3` (4051).
        function get_outbox_factory_address(uint64 chainKey)
            external
            view
            returns (address factoryAddr, bool exists);

        /// `chain-info` precompile accessor exposing the per-chain Outbox discovery-registry
        /// address (`OutboxDiscovery` in asc-contracts) registered in
        /// `SupportedChains::OutboxDiscoveries`. Unlike the factory above, this address is safe to
        /// trust directly: it is only ever written through an access-controlled deploy path, so a
        /// resolver can bind the Outbox from its `defaultOutbox` getter instead of scanning the
        /// permissionless factory's `OutboxCreated` logs. `exists` is false when no registry has
        /// been set for `chainKey`.
        function get_outbox_discovery_address(uint64 chainKey)
            external
            view
            returns (address discoveryAddr, bool exists);
    }

    #[sol(rpc)]
    #[derive(Debug)]
    contract IOutboxDiscovery {
        /// The default Outbox for `chainKey`, or the zero address if none — the source of truth
        /// asc-contracts#38 confirmed for discovery (Slack, 28 Aug 2026): "the default deployed
        /// outbox for each chain via: defaultOutbox(chainKey) (not from deployer) because there
        /// will be multiple version[s] of outbox". Written only through `registerOutbox`/
        /// `setDefaultOutbox` (owner or authorized-deployer gated).
        function defaultOutbox(uint32 chainKey) external view returns (address);
    }

    #[sol(rpc)]
    #[derive(Debug)]
    contract IVoteValidator {
        /// Active attestor EVM addresses for this validator. Queried once at startup when the
        /// attestor set is sourced from the on-chain validator.
        function attestors() external view returns (address[] memory);

        /// Quorum threshold (e.g. 2N/3 + 1). Mirrored locally so callers do not burn gas on
        /// transactions that are guaranteed to revert.
        function threshold() external view returns (uint256);

        /// Monotonic nonce bound into the attestor-set-update digest (replay/rollback protection).
        /// Attestors read it to sign the current update; it increments on each successful update.
        function attestorSetUpdateNonce() external view returns (uint256);

        /// Rotate the attestor set. `signatures` is the concatenation of 65-byte `(r,s,v)` ECDSA
        /// signatures by the *current* set over the update digest (see
        /// [`attestor_set_update_digest`](crate::hash::attestor_set_update_digest)); the contract
        /// verifies threshold-many and swaps in `newAttestors`. Permissionless — the relayer submits
        /// it once it has aggregated a threshold of gossiped signatures.
        function submitAttestorSetUpdate(address[] memory newAttestors, bytes memory signatures) external;
    }

}
