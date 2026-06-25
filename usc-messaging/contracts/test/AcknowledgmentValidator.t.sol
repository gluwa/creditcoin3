// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {AcknowledgmentValidator, IAckOutbox} from "../src/AcknowledgmentValidator.sol";
import {INativeQueryVerifier} from "../src/INativeQueryVerifier.sol";
import {EvmV1Decoder} from "../src/EvmV1Decoder.sol";

/// Minimal foundry cheatcode surface (this project has no `lib/forge-std`).
interface Vm {
    function etch(address who, bytes calldata code) external;
    function expectRevert() external;
}

/// Stand-in for the block-prover precompile that always verifies (etched at 0x…0FD2).
contract PassingVerifier is INativeQueryVerifier {
    function verify(uint64, uint64, bytes calldata, MerkleProof calldata, ContinuityProof calldata)
        external
        pure
        returns (bool)
    {
        return true;
    }
}

/// Precompile stand-in that rejects the proof.
contract RejectingVerifier is INativeQueryVerifier {
    function verify(uint64, uint64, bytes calldata, MerkleProof calldata, ContinuityProof calldata)
        external
        pure
        returns (bool)
    {
        revert("proof invalid");
    }
}

/// Records acknowledged messageIds (the Outbox this validator drives).
contract MockOutbox is IAckOutbox {
    bytes32[] public acked;

    function acknowledgeMessage(bytes32 messageId) external {
        acked.push(messageId);
    }

    function count() external view returns (uint256) {
        return acked.length;
    }
}

contract AcknowledgmentValidatorTest {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);
    address constant PRECOMPILE = 0x0000000000000000000000000000000000000FD2;

    uint64 constant CHAIN_KEY = 2;
    bytes32 constant MID = bytes32(uint256(0xABCDEF));

    AcknowledgmentValidator validator;
    MockOutbox outbox;

    function setUp() public {
        validator = new AcknowledgmentValidator(CHAIN_KEY, address(this));
        outbox = new MockOutbox();
        validator.setOutbox(address(outbox));
        // Default: a passing precompile.
        vm.etch(PRECOMPILE, address(new PassingVerifier()).code);
    }

    // --- fixture builders ----------------------------------------------------------------------- //

    /// Build prover `txBytes` (`abi.encode(uint8 txType, bytes[] chunks)`) for a type-0 tx whose
    /// receipt contains a single log with `topic0 == eventSig` and `topic1 == messageId`.
    function _txWithLog(bytes32 eventSig, bytes32 messageId) internal pure returns (bytes memory) {
        return _txWithLogData(eventSig, messageId, bytes(""));
    }

    /// As `_txWithLog`, but with an arbitrary log `data` payload — used to pad the encoded
    /// transaction to a target total size in the size-limit tests.
    function _txWithLogData(bytes32 eventSig, bytes32 messageId, bytes memory logData)
        internal
        pure
        returns (bytes memory)
    {
        bytes32[] memory topics = new bytes32[](2);
        topics[0] = eventSig;
        topics[1] = messageId;
        EvmV1Decoder.LogEntryTuple[] memory logs = new EvmV1Decoder.LogEntryTuple[](1);
        logs[0] = EvmV1Decoder.LogEntryTuple({
            address_: address(0xBEEF), topics: topics, data: logData
        });
        bytes memory receiptChunk = abi.encode(uint8(1), uint64(21000), logs, bytes(""));
        bytes[] memory chunks = new bytes[](3); // type 0–2: 3 chunks, receipt at index 2
        chunks[0] = bytes("");
        chunks[1] = bytes("");
        chunks[2] = receiptChunk;
        return abi.encode(uint8(0), chunks);
    }

    function _emptyMerkle() internal pure returns (INativeQueryVerifier.MerkleProof memory) {
        return INativeQueryVerifier.MerkleProof({
            root: bytes32(0), siblings: new INativeQueryVerifier.MerkleProofEntry[](0)
        });
    }

    function _emptyContinuity()
        internal
        pure
        returns (INativeQueryVerifier.ContinuityProof memory)
    {
        return INativeQueryVerifier.ContinuityProof({
            lowerEndpointDigest: bytes32(0), roots: new bytes32[](0)
        });
    }

    // --- tests ---------------------------------------------------------------------------------- //

    function test_acknowledges_delivered_message() public {
        bytes memory enc = _txWithLog(validator.MESSAGE_DELIVERED_SIG(), MID);
        validator.submitAcknowledgment(100, enc, _emptyMerkle(), _emptyContinuity());
        require(outbox.count() == 1, "exactly one ack");
        require(outbox.acked(0) == MID, "messageId acknowledged");
    }

    function test_reverts_when_proof_fails() public {
        vm.etch(PRECOMPILE, address(new RejectingVerifier()).code);
        bytes memory enc = _txWithLog(validator.MESSAGE_DELIVERED_SIG(), MID);
        vm.expectRevert();
        validator.submitAcknowledgment(100, enc, _emptyMerkle(), _emptyContinuity());
    }

    function test_reverts_when_no_delivered_logs() public {
        // A proven tx whose only log is some other event → nothing to acknowledge.
        bytes memory enc = _txWithLog(keccak256("SomethingElse(uint256)"), MID);
        vm.expectRevert();
        validator.submitAcknowledgment(100, enc, _emptyMerkle(), _emptyContinuity());
    }

    function test_reverts_when_outbox_not_set() public {
        AcknowledgmentValidator fresh = new AcknowledgmentValidator(CHAIN_KEY, address(this));
        bytes memory enc = _txWithLog(fresh.MESSAGE_DELIVERED_SIG(), MID);
        vm.expectRevert();
        fresh.submitAcknowledgment(100, enc, _emptyMerkle(), _emptyContinuity());
    }

    function test_accepts_encoded_transaction_at_max_size() public {
        bytes32 sig = validator.MESSAGE_DELIVERED_SIG();
        uint256 maxSize = validator.MAX_ENCODED_TRANSACTION_BYTES();
        // Pad the log data so the *total* encoded transaction is exactly the cap. Both the base
        // length and maxSize are 32-byte aligned (abi.encode output always is) and we pad by a
        // multiple of 32, so the total grows by exactly the pad amount.
        bytes memory base = _txWithLog(sig, MID);
        require(base.length <= maxSize, "base exceeds cap");
        bytes memory enc = _txWithLogData(sig, MID, new bytes(maxSize - base.length));
        require(enc.length == maxSize, "not exactly max size");
        // Exactly at the cap is accepted (the bound is inclusive) and acknowledges normally.
        validator.submitAcknowledgment(100, enc, _emptyMerkle(), _emptyContinuity());
        require(outbox.count() == 1, "exactly one ack at max size");
        require(outbox.acked(0) == MID, "messageId acknowledged");
    }

    function test_reverts_when_encoded_transaction_too_large() public {
        // One byte over the cap is rejected up front, before any proof verification/decoding,
        // so the contents are irrelevant.
        bytes memory tooBig = new bytes(validator.MAX_ENCODED_TRANSACTION_BYTES() + 1);
        vm.expectRevert();
        validator.submitAcknowledgment(100, tooBig, _emptyMerkle(), _emptyContinuity());
    }
}
