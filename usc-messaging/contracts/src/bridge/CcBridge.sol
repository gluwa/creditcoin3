// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {INativeQueryVerifier, NativeQueryVerifierLib} from "../INativeQueryVerifier.sol";
import {EvmV1Decoder} from "@gluwa/usc-contracts/decoding/EvmV1Decoder.sol";

interface IERC20 {
    function transfer(address to, uint256 amount) external returns (bool);
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
}

interface IBridgeOutbox {
    function publishMessage(bool requiresAck, bytes calldata payload) external returns (bytes32);
}

/// @notice Creditcoin-side endpoint of the 2-way USC token bridge.
///
/// **Outbound (Creditcoin → Anvil), vote-based:** `withdraw` escrows tokens and publishes a USC
/// release message via the `Outbox`; attestors vote and the relayer delivers it to
/// `AnvilBridge.receiveMessage`.
///
/// **Inbound (Anvil → Creditcoin), proof-based:** `claim` verifies a native USC proof (block-prover
/// precompile: merkle inclusion + continuity) that an `AnvilBridge.Locked` event was emitted in a
/// finalized Anvil block, decodes it, and pays out bridged liquidity. Permissionless and
/// self-validating — the same machinery as `AcknowledgmentValidator`, releasing tokens instead of
/// acknowledging a message.
contract CcBridge {
    /// `keccak256("Locked(address,uint256,uint256)")` — topic0 of `AnvilBridge.Locked`.
    bytes32 public constant LOCKED_SIG = keccak256("Locked(address,uint256,uint256)");
    /// Upper bound on proven `encodedTransaction` calldata (mirrors AcknowledgmentValidator).
    uint256 public constant MAX_ENCODED_TRANSACTION_BYTES = 500_000;

    IERC20 public immutable token;
    IBridgeOutbox public immutable outbox;
    /// Expected emitter of `Locked` on the Anvil side; claims for logs from any other address revert.
    address public immutable anvilBridge;
    /// Destination chain key the Anvil `Locked` proofs are proven against (Anvil = 2 locally).
    uint64 public immutable anvilChainKey;

    /// Anvil → CC claim dedup, keyed by `keccak256(ccRecipient, amount, nonce)`.
    mapping(bytes32 => bool) public claimed;

    event Withdrawn(address indexed anvilRecipient, uint256 amount, bytes32 messageId);
    event Claimed(address indexed ccRecipient, uint256 amount, uint256 nonce);

    error EncodedTransactionTooLarge(uint256 size, uint256 maxSize);
    error ProofVerificationFailed();
    error UnsupportedTxType(uint8 txType);
    error NoLockedLogs();
    error MalformedLockedLog();
    error WrongEmitter(address emitter);
    error AlreadyClaimed();

    constructor(address _token, address _outbox, address _anvilBridge, uint64 _anvilChainKey) {
        require(
            _token != address(0) && _outbox != address(0) && _anvilBridge != address(0), "zero arg"
        );
        token = IERC20(_token);
        outbox = IBridgeOutbox(_outbox);
        anvilBridge = _anvilBridge;
        anvilChainKey = _anvilChainKey;
    }

    /// Escrow `amount` on Creditcoin and publish a USC release message toward `anvilRecipient`. The
    /// caller must have approved this contract for `amount` first.
    function withdraw(uint256 amount, address anvilRecipient) external returns (bytes32 messageId) {
        require(anvilRecipient != address(0), "zero recipient");
        require(token.transferFrom(msg.sender, address(this), amount), "transferFrom");

        // The Inbox decodes the published payload as `(destinationContract, innerPayload)` and
        // forwards `innerPayload` to `destinationContract.receiveMessage`. Here that's
        // `AnvilBridge` and `(anvilRecipient, amount)`.
        bytes memory inner = abi.encode(anvilRecipient, amount);
        bytes memory payload = abi.encode(anvilBridge, inner);
        messageId = outbox.publishMessage(false, payload);
        emit Withdrawn(anvilRecipient, amount, messageId);
    }

    /// Prove an Anvil `Locked` event and release the bridged tokens to its `ccRecipient` on
    /// Creditcoin. Idempotent per `(ccRecipient, amount, nonce)`.
    function claim(
        uint64 height,
        bytes calldata encodedTransaction,
        INativeQueryVerifier.MerkleProof calldata merkleProof,
        INativeQueryVerifier.ContinuityProof calldata continuityProof
    ) external {
        if (encodedTransaction.length > MAX_ENCODED_TRANSACTION_BYTES) {
            revert EncodedTransactionTooLarge(
                encodedTransaction.length, MAX_ENCODED_TRANSACTION_BYTES
            );
        }

        // 1. Verify inclusion in a finalized Anvil block via the block-prover precompile.
        bool ok = NativeQueryVerifierLib.getVerifier()
            .verify(anvilChainKey, height, encodedTransaction, merkleProof, continuityProof);
        if (!ok) revert ProofVerificationFailed();

        // 2. Decode the proven tx's receipt and pull out the Locked logs.
        uint8 txType = EvmV1Decoder.getTransactionType(encodedTransaction);
        if (!EvmV1Decoder.isValidTransactionType(txType)) revert UnsupportedTxType(txType);
        EvmV1Decoder.ReceiptFields memory receipt =
            EvmV1Decoder.decodeReceiptFields(encodedTransaction);
        EvmV1Decoder.LogEntry[] memory logs =
            EvmV1Decoder.getLogsByEventSignature(receipt, LOCKED_SIG);
        if (logs.length == 0) revert NoLockedLogs();

        // 3. Release liquidity for each proven lock.
        for (uint256 i; i < logs.length; i++) {
            EvmV1Decoder.LogEntry memory log = logs[i];
            if (log.topics.length < 2) revert MalformedLockedLog();
            // Only honor Locked events from the configured AnvilBridge — otherwise an attacker could
            // mint a same-shaped event from another contract and drain liquidity.
            if (log.address_ != anvilBridge) revert WrongEmitter(log.address_);

            // Locked(address indexed ccRecipient, uint256 amount, uint256 nonce)
            address ccRecipient = address(uint160(uint256(log.topics[1])));
            (uint256 amount, uint256 nonce) = abi.decode(log.data, (uint256, uint256));

            bytes32 key = keccak256(abi.encode(ccRecipient, amount, nonce));
            if (claimed[key]) revert AlreadyClaimed();
            claimed[key] = true;

            require(token.transfer(ccRecipient, amount), "transfer");
            emit Claimed(ccRecipient, amount, nonce);
        }
    }
}
