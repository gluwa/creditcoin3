//! Solidity ABI bindings for the USC write-ability contracts.
//!
//! Shared by the attestor (which decodes `MessagePublished` from the Creditcoin Outbox) and the
//! `message-relayer` (which additionally calls `Inbox.deliverMessage` / `validateVotes`). Keeping
//! one definition here means both crates decode the *same* event signature and recompute the
//! *same* `messageHash` — a mismatch would make every signature verify as invalid on-chain.
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
        /// `emitterAddress` is the dApp that called `publishMessage`. `requiresAck` flags
        /// whether the message must be acknowledged on-chain before it is considered complete.
        /// `payload` is the opaque bytes the inbox will hand to the destination dApp's
        /// `receiveMessage`.
        event MessagePublished(
            bytes32 indexed messageId,
            address indexed emitterAddress,
            bool requiresAck,
            bytes payload
        );
    }

    #[sol(rpc)]
    #[derive(Debug)]
    contract IInbox {
        /// Submit an aggregated set of attestor votes that prove `messageId` was finalized
        /// on Creditcoin. Calldata is byte-identical to what attestors signed.
        function deliverMessage(
            bytes32 messageId,
            address emitterAddress,
            bytes calldata payload,
            bytes calldata votes
        ) external;

        /// Retry a message previously left in the `MessagePending` state (e.g. dApp ran out
        /// of gas during `receiveMessage`). Permissionless.
        function retryPendingMessage(bytes32 messageId) external;

        /// Pure check used by the relayer to simulate before paying gas. Reverts if the votes
        /// are malformed, below threshold, or signed by unauthorized signers.
        function validateVotes(bytes32 messageHash, bytes calldata votes)
            external
            view
            returns (bool);

        event MessageDelivered(bytes32 indexed messageId);
        event MessagePending(bytes32 indexed messageId);

        /// Reverts emitted by the inbox when delivery fails or is redundant. Used to classify
        /// transaction outcomes for metrics + retry logic.
        error MessageAlreadyValidated();
        error InvalidVotes();
        error VotesBelowThreshold();
    }

    #[sol(rpc)]
    #[derive(Debug)]
    contract IOutboxFactory {
        /// Resolve the per-destination Outbox instance for a USC chain key. The factory creates
        /// one Outbox per `bytes32 chainKey`; attestors call this to discover the address to watch.
        /// Returns `address(0)` when no outbox has been created for `chainKey` yet.
        function getOutbox(bytes32 chainKey) external view returns (address);

        /// @notice Emitted when a new outbox is created
        event OutboxCreated(bytes32 indexed chainKey, address indexed outboxAddress);
    }

    #[sol(rpc)]
    #[derive(Debug)]
    contract IChainInfo {
        /// `chain-info` precompile accessor (PR #873) exposing the per-chain Outbox factory
        /// address registered in `SupportedChains::OutboxFactories`. `exists` is false when no
        /// factory has been set for `chainKey`. Precompile address: `0x…0fD3` (4051).
        function outbox_factory_address(uint64 chainKey)
            external
            view
            returns (address factory_addr, bool exists);
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
    }
}
