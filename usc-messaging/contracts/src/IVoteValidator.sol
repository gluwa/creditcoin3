// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @notice Interface for vote-validation contracts (Inversion of Control for the Inbox).
/// @dev The Inbox delegates signature/threshold checking here so the scheme (EOA/BLS/TSS) can be
/// swapped without changing the Inbox. `validateVotes` reverts on failure, succeeds otherwise.
interface IVoteValidator {
    /// @notice Validate attestor votes for a message. Reverts if invalid.
    /// @param messageHash The hash attestors signed over. The Inbox computes
    ///   `keccak256(abi.encode(messageId, emitterAddress, destinationChainKey, creditcoinChainId, payload))`.
    /// @param votes Implementation-specific encoding (EOA: `abi.encode(bytes[] signatures)`).
    function validateVotes(bytes32 messageHash, bytes calldata votes) external view;

    /// @notice The current authorized attestor EVM addresses. Read off-chain by attestors/relayers
    ///   that source their attestor set from the validator.
    function attestors() external view returns (address[] memory);

    /// @notice Quorum threshold (number of unique signatures) required for the current attestor set.
    ///   Mirrored off-chain so callers don't submit transactions guaranteed to revert.
    function threshold() external view returns (uint256);
}
