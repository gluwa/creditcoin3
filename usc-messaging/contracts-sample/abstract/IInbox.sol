// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import "./IVoteValidator.sol";

interface IInbox {
    /// @notice Metadata about message delivery
    /// @param processor Address that successfully processed the message
    /// @param blockNumber Block number when message was processed
    struct DeliveryMetadata {
        address processor;
        uint256 blockNumber;
    }

    /// @notice Delivers a message to the destination contract
    /// @param messageId The message identifier
    /// @param emitterAddress The address of the contract that originally sent the message
    /// @param payload The message payload, the payload should contain the destination contract address
    /// @param votes The attestation votes from attesters
    /// @return success Whether the delivery was successful
    /// @dev Attesters sign over destinationChainKey (chain-agnostic) and creditcoinChainId
    /// @dev Chain identifiers are implicit - inbox uses destinationChainKey and creditcoinChainId when validating votes
    /// @dev This prevents cross-chain replay attacks - messages signed for wrong chain will fail validation
    function deliverMessage(
        bytes32 messageId,
        address emitterAddress,
        bytes calldata payload,
        bytes calldata votes
    ) external returns (bool success);

    /// @notice Retries a pending message that failed to deliver
    /// @param messageId The message identifier
    /// @dev Permissionless - anyone can retry pending messages
    /// @dev Destination address is extracted from the payload
    /// @dev If retry fails, transaction reverts (prevents infinite retry loops)
    /// @dev Only succeeds if destination contract successfully processes the message
    function retryPendingMessage(bytes32 messageId) external;

    /// @notice Returns the block number when a message was processed
    /// @param messageId The message identifier
    /// @return blockNumber The block number when message was processed, or 0 if not processed
    function processedAt(bytes32 messageId) external view returns (uint256);

    /// @notice Returns whether a message is pending retry
    /// @param messageId The message identifier
    /// @return pending True if message is pending, false otherwise
    function isPending(bytes32 messageId) external view returns (bool);

    /// @notice Returns the bridge receiver contract (ClientBridgeLiquidityOperator)
    /// @return receiver The message receiver contract address
    function bridgeReceiver() external view returns (address);

    /// @notice Updates the bridge receiver contract address
    /// @param receiver The new message receiver contract address
    function setBridgeReceiver(address receiver) external;

    /// @notice Returns the local chain key (chain-agnostic identifier)
    /// @return chainKey The chain key for this inbox
    function localChainKey() external view returns (bytes32);

    /// @notice Returns the Creditcoin chain ID this inbox accepts messages from
    /// @return chainId The Creditcoin EVM chain ID
    function creditcoinChainId() external view returns (uint256);

    /// @notice Returns the default vote validator used when recipient doesn't specify one
    /// @return validator The default vote validator address
    function defaultVoteValidator() external view returns (IVoteValidator);

    /// @notice Emitted when a message is successfully processed
    /// @param messageId The message identifier
    /// @param processor The address that processed the message
    /// @dev Only emitted when target contract execution succeeds
    event MessageDelivered(
        bytes32 indexed messageId,
        address indexed processor
    );

    /// @notice Emitted when a message fails validation
    /// @param messageId The message identifier
    event ValidationFailed(bytes32 indexed messageId);

    /// @notice Emitted when a message passes validation
    /// @param messageId The message identifier
    event ValidationSucceeded(bytes32 indexed messageId);

    /// @notice Emitted when a message delivery fails and is stored as pending
    /// @param messageId The message identifier
    /// @param destinationContract The destination contract that failed to process
    /// @dev Emitted when target contract execution fails (reverts)
    /// @dev destinationContract is indexed to allow dApps to filter events for their contract
    /// @dev Anyone can retry pending messages using retryPendingMessage()
    event MessagePending(
        bytes32 indexed messageId,
        address indexed destinationContract
    );

    /// @notice Emitted when the default vote validator is updated
    /// @param oldValidator The previous validator address
    /// @param newValidator The new validator address
    event DefaultVoteValidatorSet(
        address indexed oldValidator,
        address indexed newValidator
    );
}
