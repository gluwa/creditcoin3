// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {OutboxTypes} from "./OutboxTypes.sol";
import {RateLimitLib} from "./RateLimitLib.sol";

/// @title Outbox Storage Layout
/// @notice Persistent state for Outbox Core, upgrade-safe.
/// @dev DO NOT REORDER STORAGE. Append new fields only.
contract Storage {
    /// @dev Primary storage bucket for Outbox.
    struct OutboxState {
        uint32 chainKey;
        address validator;
        /**
         * @notice Rate-limit policy encoding and semantics
         *
         * The rate-limit policy is encoded into a single `uint128` value:
         *
         *   ┌────────────────────────────────────────────┐
         *   │ high 64 bits        │ low 64 bits           │
         *   │ maxRequests         │ windowSeconds         │
         *   └────────────────────────────────────────────┘
         *
         * Semantics:
         * - `windowSeconds` defines the duration of a sliding time window
         * - `maxRequests` defines the total number of allowed requests within that window
         * - The window resets when `windowSeconds` elapses since the first request in the window
         * - If either value is zero, rate limiting is disabled
         *
         * Example:
         * - policy = encode(10_000, 86_400)
         *   → allows 10,000 requests per 24-hour sliding window
         */
        uint128 defaultRateLimit;
        /// @notice Fee charged for publishing messages
        uint128 messageFee;
        uint256 evmChainId;
        /// @notice Sequence numbers per Universal Contract
        mapping(address => uint64) ucSequences;
        mapping(bytes32 => OutboxTypes.Message) messages;
        mapping(address => RateLimitLib.RateBucket) rateBuckets;
        uint256[32] __reserved;
    }

    /// @dev Accessor for OutboxState stored at slot 0.
    function _state() internal pure returns (OutboxState storage s) {
        assembly {
            s.slot := 0
        }
    }
}
