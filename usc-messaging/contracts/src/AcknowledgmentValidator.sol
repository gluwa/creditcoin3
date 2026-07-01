// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {INativeQueryVerifier, NativeQueryVerifierLib} from "./INativeQueryVerifier.sol";
import {EvmV1Decoder} from "@gluwa/usc-contracts/decoding/EvmV1Decoder.sol";

/// @notice The slice of the Outbox this validator drives. `acknowledgeMessage` is `onlyValidator`,
/// so this contract must be the Outbox's configured `validator`.
interface IAckOutbox {
    function acknowledgeMessage(bytes32 messageId) external;
}

/// @notice Trust-minimized acknowledgment for the USC write-ability layer (research §05 / §10).
///
/// Delivery infrastructure (the relayer) proves — via the chain's native USC proving (block-prover
/// precompile: merkle inclusion + continuity) — that a `MessageDelivered(bytes32 indexed messageId)`
/// event was emitted in a finalized block on the destination chain. This contract verifies that
/// proof, decodes the `MessageDelivered` logs from the proven transaction, and acknowledges each
/// message on the source Outbox. No attester votes are involved (the attestation chain finalizing
/// the destination block is what makes the proof possible).
///
/// Deploy/wiring: deploy this with the destination `chainKey`, create the Outbox with this contract
/// as its `validator` (so only it can acknowledge), then call `setOutbox` to point it at that Outbox.
contract AcknowledgmentValidator {
    /// `keccak256("MessageDelivered(bytes32)")` — topic0 of the destination Inbox's event.
    bytes32 public constant MESSAGE_DELIVERED_SIG = keccak256("MessageDelivered(bytes32)");

    /// Upper bound on the `encodedTransaction` calldata accepted by `submitAcknowledgment`, to bound
    /// the cost/work of proof verification and decoding. Submissions above this are rejected.
    uint256 public constant MAX_ENCODED_TRANSACTION_BYTES = 500_000;

    /// Destination chain key whose `MessageDelivered` events this validator proves (the chain the
    /// attestation network attests, and where the Inbox lives).
    uint64 public immutable destinationChainKey;

    address public owner;
    /// The source Outbox this validator acknowledges on. Set once after the Outbox is created.
    IAckOutbox public outbox;

    event Acknowledged(bytes32 indexed messageId);
    event OutboxSet(address indexed outbox);

    error NotOwner();
    error OutboxAlreadySet();
    error OutboxNotSet();
    error ProofVerificationFailed();
    error EncodedTransactionTooLarge(uint256 size, uint256 maxSize);
    error UnsupportedTxType(uint8 txType);
    error NoMessageDeliveredLogs();
    error MalformedMessageDeliveredLog();

    modifier onlyOwner() {
        if (msg.sender != owner) revert NotOwner();
        _;
    }

    constructor(uint64 _destinationChainKey, address _owner) {
        require(_owner != address(0), "owner=0");
        destinationChainKey = _destinationChainKey;
        owner = _owner;
    }

    /// @notice Point this validator at the Outbox it acknowledges (one-time; the Outbox must have
    /// been created with this contract as its `validator`).
    function setOutbox(address _outbox) external onlyOwner {
        if (address(outbox) != address(0)) revert OutboxAlreadySet();
        require(_outbox != address(0), "outbox=0");
        outbox = IAckOutbox(_outbox);
        emit OutboxSet(_outbox);
    }

    /// @notice Prove a destination transaction containing `MessageDelivered` event(s) and acknowledge
    /// each message on the source Outbox. Permissionless — the proof is self-validating.
    /// @param height Destination block height containing the transaction.
    /// @param encodedTransaction Prover `txBytes` (encoded tx + receipt) for that transaction.
    /// @param merkleProof Merkle inclusion proof of the transaction in the block.
    /// @param continuityProof Continuity proof that the attestation chain finalized the block.
    function submitAcknowledgment(
        uint64 height,
        bytes calldata encodedTransaction,
        INativeQueryVerifier.MerkleProof calldata merkleProof,
        INativeQueryVerifier.ContinuityProof calldata continuityProof
    ) external {
        if (address(outbox) == address(0)) revert OutboxNotSet();

        // Reject oversized submissions up front (cheap check before proof verification/decoding).
        if (encodedTransaction.length > MAX_ENCODED_TRANSACTION_BYTES) {
            revert EncodedTransactionTooLarge(encodedTransaction.length, MAX_ENCODED_TRANSACTION_BYTES);
        }

        // 1. Verify the transaction was included in a finalized block of the destination chain.
        bool ok = NativeQueryVerifierLib.getVerifier()
            .verify(destinationChainKey, height, encodedTransaction, merkleProof, continuityProof);
        if (!ok) revert ProofVerificationFailed();

        // 2. Decode the proven transaction's receipt and pull out the MessageDelivered logs.
        uint8 txType = EvmV1Decoder.getTransactionType(encodedTransaction);
        if (!EvmV1Decoder.isValidTransactionType(txType)) revert UnsupportedTxType(txType);
        EvmV1Decoder.ReceiptFields memory receipt =
            EvmV1Decoder.decodeReceiptFields(encodedTransaction);
        EvmV1Decoder.LogEntry[] memory logs =
            EvmV1Decoder.getLogsByEventSignature(receipt, MESSAGE_DELIVERED_SIG);
        if (logs.length == 0) revert NoMessageDeliveredLogs();

        // 3. Acknowledge each delivered message on the source Outbox. messageId is the sole indexed
        //    arg of MessageDelivered(bytes32 indexed messageId), i.e. topics[1].
        for (uint256 i; i < logs.length; i++) {
            if (logs[i].topics.length < 2) revert MalformedMessageDeliveredLog();
            bytes32 messageId = logs[i].topics[1];
            outbox.acknowledgeMessage(messageId);
            emit Acknowledged(messageId);
        }
    }
}
