// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.26;
contract TestOutbox {
    bytes32 public chainKey;
    constructor(bytes32 _chainKey) { chainKey = _chainKey; }
    event MessagePublished(bytes32 indexed messageId, bytes32 indexed emitterAddress, uint64 sequence, bool canAck, bytes payload);
    function publish(bytes32 messageId, uint64 sequence, bytes calldata payload) external {
        emit MessagePublished(messageId, bytes32(uint256(uint160(msg.sender)) << 96), sequence, true, payload);
    }
}
