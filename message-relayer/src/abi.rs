//! Solidity ABI bindings used by the relayer.
//!
//! Inline `alloy::sol!` declarations are used while the production contracts are still being
//! finalized — when they ship, switch each block to the JSON form
//! (`#[sol(rpc)] interface X, "contracts/x.json"`) following the pattern in
//! `common/eth/src/evm/block_prover.rs:36`. Keep the function & event signatures byte-identical
//! with the production artefacts; the relayer's correctness depends on the encoding matching
//! what attesters sign and what `validateVotes` recomputes.

use alloy::sol;

sol! {
    #[sol(rpc)]
    #[derive(Debug)]
    contract IOutbox {
        /// A new cross-chain message has been published to this outbox.
        ///
        /// `messageId` is the unique handle attesters and the inbox use to track delivery.
        /// `emitterAddress` is the dApp that called `publishMessage`. `payload` is the
        /// opaque bytes the inbox will hand to the destination dApp's `receiveMessage`.
        event MessagePublished(
            bytes32 indexed messageId,
            address indexed emitterAddress,
            bytes payload
        );

        /// One outbox is bound to one destination chain. `chainKey()` is that destination's
        /// USC chain key, used in `messageHash`. Read once at startup per route.
        function chainKey() external view returns (bytes32);
    }

    #[sol(rpc)]
    #[derive(Debug)]
    contract IInbox {
        /// Submit an aggregated set of attester votes that prove `messageId` was finalized
        /// on Creditcoin. Calldata is byte-identical to what attesters signed.
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
    contract IVoteValidator {
        /// Active attester EVM addresses for this validator. The relayer queries this once
        /// at startup when `attester_set: { kind: evm_contract, ... }` is configured.
        function attesters() external view returns (address[] memory);

        /// Quorum threshold (e.g. 2N/3 + 1). The relayer mirrors this locally so it does not
        /// burn gas on transactions that are guaranteed to revert.
        function threshold() external view returns (uint256);
    }
}
