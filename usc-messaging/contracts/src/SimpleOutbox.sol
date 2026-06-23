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
        bytes32 indexed messageId,
        address indexed emitterAddress,
        bool requiresAck,
        bytes payload
    );

    event MessageAcknowledged(bytes32 indexed messageId);

    // Errors
    error MessageDoesNotRequireAck(bytes32 messageId);
    error MessageNotFound(bytes32 messageId);
    error MessageAlreadyAcknowledged(bytes32 messageId);

    // Mapping of messageId to Message struct for stored messages (e.g. requiresAck = true)
    mapping(bytes32 => Message) public messages;

    // Mapping to track whether a message requires acknowledgment (messageId => requiresAck)
    mapping(bytes32 => bool) public messageRequiresAck;

    // Sequence numbers per Universal Contract used to generate nonces for message IDs
    mapping(address => uint64) public uscSequences;

    // Destination chain this outbox publishes toward. Set once by the factory at creation.
    bytes32 public chainKey;

    // Validator authorized to verify acknowledgment delivery proofs for this outbox. Supplied by
    // the factory at creation.
    // TODO(write-ability): once the acknowledgment delivery-proof flow is implemented, gate
    // `acknowledgeMessage` on this validator (see the TODO on that function).
    address public validator;

    // Owner shared with the factory — the factory passes its own owner in so the same account
    // controls the factory and every outbox it creates.
    address public owner;

    // Created by `OutboxFactory.createOutbox` (see SimpleOutboxFactory.sol). The factory passes its
    // own owner so the same account controls both.
    constructor(bytes32 _chainKey, address _validator, address _owner) {
        chainKey = _chainKey;
        validator = _validator;
        owner = _owner;
    }

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

        messageRequiresAck[messageId] = requiresAck;

        emit MessagePublished(messageId, usContract, requiresAck, payload);
    }

    // TODO(write-ability): restrict this to the configured `validator` once the acknowledgment
    // delivery-proof verification flow is implemented. It is currently permissionless — any caller
    // can acknowledge any message — which is insecure and only acceptable for the PoC.
    function acknowledgeMessage(bytes32 messageId) public {
        Message storage m = messages[messageId];

        if (m.emitter == address(0)) {
            revert MessageNotFound(messageId);
        }
        if (m.acknowledged) {
            revert MessageAlreadyAcknowledged(messageId);
        }

        if (!messageRequiresAck[messageId]) {
            revert MessageDoesNotRequireAck(messageId);
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
