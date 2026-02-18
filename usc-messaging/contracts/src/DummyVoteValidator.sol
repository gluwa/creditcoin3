// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @notice Dummy vote validator for PoC. Always accepts.
contract DummyVoteValidator {
    function validateVotes(bytes32 /* messageHash */, bytes calldata /* votes */) external pure {
        // No-op: always passes
    }
}
