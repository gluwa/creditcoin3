// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @notice Minimal stand-in for the Outbox contract's `publishMessage` entry point — records
///         what it was called with instead of doing any real fee/message-store bookkeeping.
///         BridgeHub only ever calls `publishMessage`, so this mock does not need to implement
///         the rest of IOutbox.
contract MockOutbox {
    uint64 public sequence;
    bool public canAckLast;
    bytes public payloadLast;
    address public callerLast;

    function publishMessage(bool canAck, bytes calldata payload)
        external
        returns (bytes32 messageId)
    {
        canAckLast = canAck;
        payloadLast = payload;
        callerLast = msg.sender;
        messageId = keccak256(abi.encode(address(this), msg.sender, sequence, payload));
        sequence++;
    }
}
