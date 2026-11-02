// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @notice Test destination for PoC. Receives messages and stores them.
contract TestDestination {
    struct ReceivedMessage {
        bytes32 messageId;
        uint256 creditcoinChainId;
        address emitterAddress;
        bytes payload;
    }

    ReceivedMessage[] public messages;

    event MessageReceived(bytes32 indexed messageId, address indexed emitter, bytes payload);

    function receiveMessage(
        bytes32 messageId,
        uint256 creditcoinChainId,
        address emitterAddress,
        bytes calldata payload
    ) external {
        messages.push(ReceivedMessage(messageId, creditcoinChainId, emitterAddress, payload));
        emit MessageReceived(messageId, emitterAddress, payload);
    }

    function messageCount() external view returns (uint256) {
        return messages.length;
    }
}
