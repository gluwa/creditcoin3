// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {OutboxTypes} from "../common/OutboxTypes.sol";


import {UniversalContract} from "./UniversalContract.sol";


/// @title Outbox interface (per client chain)
/// @notice Deployed on Creditcoin L1, one instance per client chain
interface IOutbox {
    /// @notice Emitted when a message is published
    event MessagePublished(
        bytes32 indexed messageId,
        bytes32 indexed emitterAddress, // Universal Contract address as bytes32 to ensure consistency across chains
        bool requiresAck,
        bytes payload
    );

    /// @notice Emitted when a message is acknowledged by the validator
    event MessageAcknowledged(bytes32 indexed messageId);

    /// @notice Emitted when validator is updated
    event ValidatorChanged(
        address indexed oldValidator,
        address indexed newValidator
    );

    function defaultRateLimit() external view returns (uint128);

    function getSequence(address dApp) external view returns (uint64);    

    /// @notice Returns the validator contract trusted to call acknowledge functions
    function validator() external view returns (address);

    /// @notice Returns the administrative owner of this outbox
    function owner() external view returns (address);

    /// @notice Returns stored metadata for a published message
    /// @dev Only messages that are stored (e.g. requiresAck = true)
    /// @param messageId The message identifier
    function getMessage(
        bytes32 messageId
    ) external view returns (OutboxTypes.Message memory);

    /// @notice Publishes a message to be relayed to the target chain
    /// @param requiresAck Whether this message requires acknowledgment
    /// @param payload The message payload (chain-specific format)
    /// @return messageId The unique identifier for this message
    function publishMessage(
        bool requiresAck,
        bytes calldata payload
    ) external returns (bytes32 messageId);

    /// @notice Acknowledges a message (only callable by validator)
    /// @param messageId The message identifier
    function acknowledgeMessage(bytes32 messageId) external;

    /// @notice Batch version of acknowledgeMessage (only callable by validator)
    /// @param messageIds Array of message identifiers
    function batchAcknowledgeMessages(bytes32[] calldata messageIds) external;

    /// @notice Returns whether a message has been acknowledged
    /// @param messageId The message identifier
    /// @return acknowledged True if message has been acknowledged
    function isAcknowledged(
        bytes32 messageId
    ) external view returns (bool acknowledged);

    /// @notice Returns whether a message was published with requiresAck = true
    /// @param messageId The message identifier
    /// @return requiresAck True if this message is tracked in storage
    function messageRequiresAck(
        bytes32 messageId
    ) external view returns (bool requiresAck);

    /// @notice Sets the validator contract that can acknowledge messages
    /// @param newValidator The new validator contract address
    /// @dev Expected to be owner-only in implementation
    function setValidator(address newValidator) external;
}
