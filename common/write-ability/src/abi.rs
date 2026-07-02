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

        /// Whether `messageId` was published with `requiresAck = true`. Mirrors the public
        /// `SimpleOutbox.messageRequiresAck` mapping; the ack submitter pre-checks this so bridge
        /// traffic (`requiresAck = false`) skips the proof fetch entirely.
        function messageRequiresAck(bytes32 messageId) external view returns (bool);

        /// Stored message state. Mirrors the public `SimpleOutbox.messages` mapping getter:
        /// `emitter == address(0)` means unknown message, `acknowledged` means the ack already
        /// landed (a duplicate submit would revert `MessageAlreadyAcknowledged`).
        function messages(bytes32 messageId)
            external
            view
            returns (address emitter, bool acknowledged, bytes32 payloadHash);

        /// Reverts bubbled up through `AcknowledgmentValidator.submitAcknowledgment` when it calls
        /// `acknowledgeMessage` here. All three are permanent for a given delivery tx — the ack
        /// submitter classifies them as terminal (see `message-relayer/src/ack`).
        error MessageDoesNotRequireAck(bytes32 messageId);
        error MessageNotFound(bytes32 messageId);
        error MessageAlreadyAcknowledged(bytes32 messageId);
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

        /// Whether `messageId` was validated but its `receiveMessage` callback failed, leaving it
        /// retryable via `retryPendingMessage`. Mirrors `SimpleInbox.isPending`.
        function isPending(bytes32 messageId) external view returns (bool);

        /// Pure check used by the relayer to simulate before paying gas. Reverts if the votes
        /// are malformed, below threshold, or signed by unauthorized signers.
        function validateVotes(bytes32 messageHash, bytes calldata votes)
            external
            view
            returns (bool);

        event MessageDelivered(bytes32 indexed messageId);
        /// Emitted (on a **successful** `deliverMessage` tx) when the votes validated but the
        /// dApp's `receiveMessage` callback reverted — the message is stored for
        /// `retryPendingMessage`. Signature must match `SimpleInbox.MessagePending` exactly or
        /// receipt-log classification silently misses it.
        event MessagePending(bytes32 indexed messageId, address indexed destinationContract);

        /// Reverts emitted by the inbox when delivery fails or is redundant. Used to classify
        /// transaction outcomes for metrics + retry logic. NOTE: the current `SimpleInbox` rejects
        /// duplicates with `require(..., "Already validated")` (a string revert) — classifiers must
        /// match that string as well as the custom-error selector kept for future inbox versions.
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

    #[sol(rpc)]
    #[derive(Debug)]
    contract IAcknowledgmentValidator {
        /// Trust-minimized acknowledgment entrypoint on the *source* (Creditcoin) chain. The relayer
        /// proves — via the chain's native USC proving (block-prover precompile: merkle inclusion +
        /// continuity) — that a `MessageDelivered` event was emitted in a finalized block on the
        /// destination chain. This contract verifies the proof, decodes the delivered messageId(s),
        /// and calls `Outbox.acknowledgeMessage`. Permissionless: the proof is self-validating.
        ///
        /// `height` is the destination block height; `encodedTransaction` is the prover `txBytes`
        /// (encoded tx + receipt); the two proof structs mirror the block-prover precompile inputs.
        function submitAcknowledgment(
            uint64 height,
            bytes calldata encodedTransaction,
            MerkleProof calldata merkleProof,
            ContinuityProof calldata continuityProof
        ) external;

        event Acknowledged(bytes32 indexed messageId);

        /// Validator-local reverts (proof rejected before reaching the Outbox). Permanent for a
        /// given proof, so the ack submitter treats them as terminal. Message-state errors
        /// (`MessageDoesNotRequireAck` / `MessageNotFound` / `MessageAlreadyAcknowledged`) bubble
        /// up from the Outbox — see [`IOutbox`].
        error ProofVerificationFailed();
        error NoMessageDeliveredLogs();
        error MalformedMessageDeliveredLog();
        error EncodedTransactionTooLarge(uint256 size, uint256 maxSize);
        error UnsupportedTxType(uint8 txType);
    }

    /// One sibling along the merkle inclusion path. `isLeft` says whether the sibling is the
    /// left-hand input when hashing up to the parent.
    #[derive(Debug)]
    struct MerkleProofEntry {
        bytes32 hash;
        bool isLeft;
    }

    /// Merkle inclusion proof of the transaction within its block's transaction trie.
    #[derive(Debug)]
    struct MerkleProof {
        bytes32 root;
        MerkleProofEntry[] siblings;
    }

    /// Continuity proof that the attestation chain finalized the destination block: the chain of
    /// block-root digests from a known lower endpoint up to the proven height.
    #[derive(Debug)]
    struct ContinuityProof {
        bytes32 lowerEndpointDigest;
        bytes32[] roots;
    }
}
