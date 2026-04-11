// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

library OutboxTypes {
    struct Message {
        address emitter;        // UC which call publishMessage in Outbox
        uint64  sequence;       // sequence to prevent duplication however, can be used to track ordering
        uint64  timestamp;      // block.timestamp at published time
        bool    requiresAck;    // requires validator ack
        bool    acknowledged;   // set by validator
        bytes32 payloadHash;    // keccak256(payload)
    }

    /// @notice Deterministic messageId derivation
    function computeMessageId(
        address outbox,
        address emitter,
        uint64 sequence,
        bytes32 payloadHash
    ) internal pure returns (bytes32) {
        return keccak256(
            abi.encode(outbox, emitter, sequence, payloadHash)
        );
    }
}
