// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

contract Outbox {
    // Message structure stored for each published message
    struct Message {
        address emitter;
        bool acknowledged;
        bytes32 payloadHash;
    }

    // Events
    event MessagePublished(
        bytes32 indexed messageId, bytes32 indexed emitterAddress, bool requiresAck, bytes payload
    );

    event MessageAcknowledged(bytes32 indexed messageId);

    // Errors
    error MessageNotFound(bytes32 messageId);
    error MessageAlreadyAcknowledged(bytes32 messageId);

    // Mapping of messageId to Message struct for stored messages (e.g. requiresAck = true)
    mapping(bytes32 => Message) public messages;

    // Sequence numbers per Universal Contract used to generate nonces for message IDs
    mapping(address => uint64) public uscSequences;

    function publishMessage(bool requiresAck, bytes calldata payload)
        external
        returns (bytes32 messageId)
    {
        address usContract = msg.sender;
        uint64 seq = uint64(++uscSequences[usContract]);
        bytes32 payloadHash = keccak256(payload);

        messageId = keccak256(abi.encode(address(this), usContract, seq, payloadHash));

        messages[messageId] =
            Message({emitter: usContract, acknowledged: false, payloadHash: payloadHash});

        emit MessagePublished(messageId, bytes32(bytes20(usContract)), requiresAck, payload);
    }

    function acknowledgeMessage(bytes32 messageId) public {
        Message storage m = messages[messageId];

        if (m.emitter == address(0)) {
            revert MessageNotFound(messageId);
        }
        if (m.acknowledged) {
            revert MessageAlreadyAcknowledged(messageId);
        }

        m.acknowledged = true;
        emit MessageAcknowledged(messageId);
    }

    function batchAcknowledgeMessages(bytes32[] calldata messageIds) external {
        for (uint256 i = 0; i < messageIds.length; ++i) {
            acknowledgeMessage(messageIds[i]);
        }
    }
}
