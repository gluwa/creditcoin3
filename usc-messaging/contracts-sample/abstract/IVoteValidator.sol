// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @title IVoteValidator
/// @notice Interface for vote validation contracts using Inversion of Control pattern
/// @dev Allows flexible signature verification schemes without modifying inbox contract
interface IVoteValidator {
    /// @notice Validates attestation votes for a message
    /// @param messageHash The message hash that attesters signed over
    /// @param votes The attestation votes from attesters
    /// @dev Inbox computes messageHash = keccak256(abi.encode(messageId, emitterAddress, destinationChainKey, creditcoinChainId, payload))
    /// @dev Validator only needs to verify signatures match the hash - doesn't need chain identifiers
    /// @dev Implementation can use any signature scheme: ECDSA (for EoA), TSS, BLS, etc.
    /// @return ok True if validation passes, false otherwise
    function validateVotes(
        bytes32 messageHash,
        bytes calldata votes
    ) external view returns (bool);

    /// @notice Returns the type of validator (for introspection)
    /// @return validatorType String identifier for the validator type 
    function validatorType() external view returns (string memory);
}
