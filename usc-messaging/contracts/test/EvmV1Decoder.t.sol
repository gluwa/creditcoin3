// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {EvmV1Decoder} from "@gluwa/usc-contracts/decoding/EvmV1Decoder.sol";

/// Direct coverage of the decoder path the AcknowledgmentValidator relies on: tx-type extraction,
/// validity check, and receipt/log decoding + filtering from prover `txBytes`
/// (`abi.encode(uint8 txType, bytes[] chunks)`).
contract EvmV1DecoderTest {
    bytes32 constant DELIVERED_SIG = keccak256("MessageDelivered(bytes32)");
    bytes32 constant OTHER_SIG = keccak256("SomethingElse()");
    bytes32 constant DELIVERED_ID = bytes32(uint256(0xabc));

    /// Build a minimal well-formed type-0/2 `txBytes`: 3 chunks (common, type-specific, receipt).
    /// The receipt carries two logs — one `MessageDelivered`, one unrelated — to exercise filtering.
    function _encode(uint8 txType) internal pure returns (bytes memory) {
        bytes memory common =
            abi.encode(uint64(1), uint64(21000), address(0x1), false, address(0x2), uint256(0), bytes(""));
        // Type-specific shape differs per type, but the receipt path never reads it; a legacy
        // (uint128 gasPrice, uint256 v, bytes32 r, bytes32 s) tuple is a fine filler.
        bytes memory typeSpecific = abi.encode(uint128(1), uint256(27), bytes32(0), bytes32(0));

        EvmV1Decoder.LogEntryTuple[] memory logs = new EvmV1Decoder.LogEntryTuple[](2);

        bytes32[] memory deliveredTopics = new bytes32[](2);
        deliveredTopics[0] = DELIVERED_SIG;
        deliveredTopics[1] = DELIVERED_ID;
        logs[0] = EvmV1Decoder.LogEntryTuple({address_: address(0x3), topics: deliveredTopics, data: hex"00"});

        bytes32[] memory otherTopics = new bytes32[](1);
        otherTopics[0] = OTHER_SIG;
        logs[1] = EvmV1Decoder.LogEntryTuple({address_: address(0x4), topics: otherTopics, data: hex""});

        bytes memory receipt = abi.encode(uint8(1), uint64(50000), logs, bytes(""));

        bytes[] memory chunks = new bytes[](3);
        chunks[0] = common;
        chunks[1] = typeSpecific;
        chunks[2] = receipt;

        return abi.encode(txType, chunks);
    }

    function test_get_transaction_type() public pure {
        require(EvmV1Decoder.getTransactionType(_encode(0)) == 0, "type 0");
        require(EvmV1Decoder.getTransactionType(_encode(2)) == 2, "type 2");
    }

    function test_is_valid_transaction_type() public pure {
        require(EvmV1Decoder.isValidTransactionType(0), "0 valid");
        require(EvmV1Decoder.isValidTransactionType(4), "4 valid");
        require(!EvmV1Decoder.isValidTransactionType(5), "5 invalid");
    }

    function test_decode_receipt_fields() public pure {
        EvmV1Decoder.ReceiptFields memory r = EvmV1Decoder.decodeReceiptFields(_encode(0));
        require(r.receiptStatus == 1, "status");
        require(r.receiptGasUsed == 50000, "gasUsed");
        require(r.receiptLogs.length == 2, "both logs decoded");
    }

    function test_filter_logs_by_event_signature() public pure {
        EvmV1Decoder.ReceiptFields memory r = EvmV1Decoder.decodeReceiptFields(_encode(0));
        EvmV1Decoder.LogEntry[] memory matched = EvmV1Decoder.getLogsByEventSignature(r, DELIVERED_SIG);
        require(matched.length == 1, "only the MessageDelivered log matches");
        require(matched[0].topics.length == 2, "topics preserved");
        require(matched[0].topics[1] == DELIVERED_ID, "indexed messageId preserved");
    }

    function test_filter_returns_empty_when_no_match() public pure {
        EvmV1Decoder.ReceiptFields memory r = EvmV1Decoder.decodeReceiptFields(_encode(0));
        EvmV1Decoder.LogEntry[] memory matched =
            EvmV1Decoder.getLogsByEventSignature(r, keccak256("Unrelated(uint256)"));
        require(matched.length == 0, "no logs match an absent signature");
    }
}
