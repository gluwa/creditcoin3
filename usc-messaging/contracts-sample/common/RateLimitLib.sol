// SPDX-License-Identifier: MIT
pragma solidity >=0.8.0 <0.9.0;

/// @notice Message rate limits with fully configurable windows
library RateLimitLib {
    error RateLimitExceeded(uint256 allowed, uint256 attempted);

    /// @dev Tracks usage within a window
    struct RateBucket {
        uint64 windowStart;
        uint64 used;
    }

    /// @notice Maximum requests allowed in the window
    function maxRequests(uint128 policy) internal pure returns (uint64) {
        return uint64(policy >> 64);
    }

    /// @notice Window size in seconds
    function windowSeconds(uint128 policy) internal pure returns (uint64) {
        return uint64(policy);
    }

    /// @notice Enforces total requests within the configured window
    /// @dev Window size is fully caller-controlled
    function enforce(
        RateBucket storage bucket,
        uint128 policy
    ) internal {
        uint64 limit = maxRequests(policy);
        uint64 window = windowSeconds(policy);

        if (limit == 0 || window == 0) return;

        uint64 nowTs = uint64(block.timestamp);

        // reset window when expired
        if (bucket.windowStart == 0 || nowTs >= bucket.windowStart + window) {
            bucket.windowStart = nowTs;
            bucket.used = 0;
        }

        if (bucket.used >= limit) {
            revert RateLimitExceeded(limit, bucket.used + 1);
        }

        unchecked {
            bucket.used += 1;
        }
    }

    /// @notice Encodes a rate policy
    function encode(
        uint64 maxRequests_,
        uint64 windowSeconds_
    ) internal pure returns (uint128) {
        return (uint128(maxRequests_) << 64) | uint128(windowSeconds_);
    }
}