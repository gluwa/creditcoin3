// This contract plays the role of a dApp contract deployed on Creditcoin
// by a builder team. DApp contracts could trigger message passing via 
// writability for many reasons, but in our example we model the simplest
// possible reason. An end user makes a dApp contract call directly 
// requesting cross chain messaging.

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

interface IOutbox {
    function publishMessage(
        bool requiresAck,
        bytes calldata payload
    ) external returns (bytes32 messageId);
}

contract SimpleDApp {
    address public owner;
    address public outboxAddr;
    IOutbox public outbox;

    mapping(bytes32 => bool) public messagePublished;
    mapping(bytes32 => bool) public messageDelivered;

    event MessageDelivered(bytes32 indexed messageId);

    modifier onlyOwner() {
        require(msg.sender == owner, "Not owner");
        _;
    }

    constructor(address _outboxAddr) {
        require(_outboxAddr != address(0), "Invalid outbox address");
        owner = msg.sender;
        outboxAddr = _outboxAddr;
        outbox = IOutbox(_outboxAddr);
    }

    function publishMessage(
        bool requiresAck,
        address destinationContract,
        string calldata message
    ) external returns (bytes32 messageId) {
        bytes memory payloadData = bytes(message);
        bytes memory payload = abi.encode(destinationContract, payloadData);

        messageId = outbox.publishMessage(requiresAck, payload);

        messagePublished[messageId] = true;
    }

    function markDelivered(bytes32 messageId) external onlyOwner {
        require(messagePublished[messageId], "Unknown messageId");
        messageDelivered[messageId] = true;

        emit MessageDelivered(messageId);
    }
}