// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {IClientBridgeLiquidityOperator} from "./abstract/IClientBridgeLiquidityOperator.sol";
import {IBridgeMessageFormat} from "./abstract/IBridgeMessageFormat.sol";
import {MessageReceiverBase} from "./abstract/MessageReceiverBase.sol";
import {ClientBridgeLiquidityOperatorErrors} from "./error/ClientBridgeLiquidityOperatorErrors.sol";

contract ClientBridgeLiquidityOperator is
    MessageReceiverBase,
    IClientBridgeLiquidityOperator
{
    event RemoteOperatorUpdated(
        uint16 indexed chainId,
        address indexed operator
    );
    event AuthorizedChainUpdated(uint16 indexed chainId, bool authorized);
    event BridgeMessageExecuted(
        bytes32 indexed intentId,
        address indexed destinationAddress
    );
    event BridgeMessageExecutionTracked(
        bytes32 indexed intentId,
        bool success,
        uint256 gasUsed,
        bytes32 returnedDataHash
    );
    event MessageFormatUpdated(address indexed formatContract);

    mapping(uint16 => address) public remoteUSCBridgeOperators;
    mapping(uint16 => bool) public authorizedCreditcoinChainIds;
    mapping(bytes32 => bool) public processedIntentIds;
    IBridgeMessageFormat public messageFormat;

    /// @notice Creates the operator with initial inbox and owner.
    /// @dev Inbound delivery is handled through MessageReceiverBase.receiveMessage.
    constructor(
        address initialInbox,
        address initialOwner,
        address initialMessageFormat
    ) MessageReceiverBase(initialInbox, initialOwner) {
        if (initialMessageFormat == address(0)) {
            revert ClientBridgeLiquidityOperatorErrors.ZeroAddress();
        }

        messageFormat = IBridgeMessageFormat(initialMessageFormat);
    }

    /// @notice Unsupported in the current implementation.
    /// @dev Keep only for interface compatibility.
    function receiveBridgeMessage(
        uint16,
        bytes32,
        bytes calldata
    ) external pure override {
        revert ClientBridgeLiquidityOperatorErrors.FunctionNotImplemented();
    }

    /// @notice Unsupported in the current implementation.
    /// @dev Keep only for interface compatibility.
    function bridgeTo(
        uint16,
        bytes32,
        uint8,
        bytes calldata
    ) external pure override {
        revert ClientBridgeLiquidityOperatorErrors.FunctionNotImplemented();
    }

    /// @notice Sets the trusted USC bridge operator for a source chain.
    /// @param chainId chain ID expected on inbound messages.
    /// @param operator Remote USCBridgeLiquidityOperator address.
    function setRemoteUSCBridgeOperator(
        uint16 chainId,
        address operator
    ) external onlyOwner {
        if (operator == address(0)) {
            revert ClientBridgeLiquidityOperatorErrors.ZeroAddress();
        }

        remoteUSCBridgeOperators[chainId] = operator;
        emit RemoteOperatorUpdated(chainId, operator);
    }

    /// @notice Authorizes or revokes a source Creditcoin chain ID.
    /// @param chainId Creditcoin chain ID.
    /// @param authorized True to allow messages from this chain.
    function setAuthorizedCreditcoinChainId(
        uint16 chainId,
        bool authorized
    ) external onlyOwner {
        authorizedCreditcoinChainIds[chainId] = authorized;
        emit AuthorizedChainUpdated(chainId, authorized);
    }

    /// @notice Sets the decoder contract used for inbound payload format.
    /// @param newMessageFormat Address of the format/decoder contract.
    function setMessageFormat(address newMessageFormat) external onlyOwner {
        if (newMessageFormat == address(0)) {
            revert ClientBridgeLiquidityOperatorErrors.ZeroAddress();
        }

        messageFormat = IBridgeMessageFormat(newMessageFormat);
        emit MessageFormatUpdated(newMessageFormat);
    }

    /// @notice Returns whether a messageId has been successfully processed.
    /// @param messageId Message identifier delivered by the inbox.
    function isMessageProcessed(bytes32 messageId) external view returns (bool) {
        return processedMessages[messageId];
    }

    /// @inheritdoc MessageReceiverBase
    /// @dev Validates sourceChainId can be safely cast to uint16.
    function _processMessage(
        bytes32,
        uint256 sourceChainId,
        address emitterAddress,
        bytes calldata payload
    ) internal override {
        if (sourceChainId > type(uint16).max) {
            revert ClientBridgeLiquidityOperatorErrors.InvalidChainId(
                sourceChainId
            );
        }

        _handleBridgeMessage(uint16(sourceChainId), emitterAddress, payload);
    }

    /// @notice Validates and executes an inbound bridge message.
    /// @dev On destination execution failure this function does not revert;
    /// it tracks failure metadata and leaves intent unprocessed for retry.
    function _handleBridgeMessage(
        uint16 sourceChainId,
        address emitterAddress,
        bytes calldata payload
    ) internal {
        if (!authorizedCreditcoinChainIds[sourceChainId]) {
            revert ClientBridgeLiquidityOperatorErrors.UnauthorizedChain(
                sourceChainId
            );
        }
        if (
            remoteUSCBridgeOperators[sourceChainId] == address(0) ||
            emitterAddress != remoteUSCBridgeOperators[sourceChainId]
        ) {
            revert ClientBridgeLiquidityOperatorErrors.UnauthorizedEmitter(
                emitterAddress
            );
        }

        (bytes32 intentId, address destinationAddress, bytes memory callData) = IBridgeMessageFormat(
            messageFormat
        ).decodeBridgePayload(payload);

        if (destinationAddress == address(0)) {
            revert ClientBridgeLiquidityOperatorErrors.EmptyDestinationAddress();
        }

        if (processedIntentIds[intentId]) {
            revert ClientBridgeLiquidityOperatorErrors.IntentAlreadyProcessed(
                intentId
            );
        }

        (bool success, bytes memory returnedData, uint256 gasUsed) = _callDestination(
            destinationAddress,
            callData
        );

        emit BridgeMessageExecutionTracked(
            intentId,
            success,
            gasUsed,
            keccak256(returnedData)
        );

        if (success) {
            processedIntentIds[intentId] = true;
            emit BridgeMessageExecuted(intentId, destinationAddress);
        }
    }

    /// @notice Executes the destination contract call and tracks runtime cost.
    function _callDestination(
        address destinationAddress,
        bytes memory callData
    ) internal returns (bool success, bytes memory retData, uint256 gasUsed) {
        uint256 gasBefore = gasleft();
        (success, retData) = destinationAddress.call(callData);
        gasUsed = gasBefore - gasleft();
    }

}
