// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import "./IVoteValidator.sol";

/// @title IMessageReceiver
/// @notice Interface that all message recipient contracts (dApps) must implement
interface IMessageReceiver {
    error UnauthorizedInbox(address caller);
    /// @notice Handles incoming cross-chain messages
    /// @param messageId The unique message identifier
    /// @param sourceChainId The Creditcoin chain ID where the message originated
    /// @param emitterAddress The address of the contract that sent the message
    /// @param payload The message payload (dApp-specific data) , the payload should contain the destination contract address
    /// @dev This function is called by the inbox contract when delivering messages
    /// @dev MUST NOT revert if you want to support automatic retry via pending messages
    /// @dev If this reverts, message will be stored as pending for manual retry
    function receiveMessage(
        bytes32 messageId,
        uint256 sourceChainId,
        address emitterAddress,
        bytes calldata payload
    ) external;

    /// @notice Trust or untrust an inbox contract
    /// @param inbox The inbox address
    /// @param trusted Whether the inbox is trusted
    /// @dev Authorization (owner / admin) is implementation-defined
    /// @dev Allows coexistence of Inbox v1 and v2
    function setTrustedInbox(address inbox, bool trusted) external;

    /// @notice Returns whether an inbox is trusted
    function isTrustedInbox(address inbox) external view returns (bool);
}
