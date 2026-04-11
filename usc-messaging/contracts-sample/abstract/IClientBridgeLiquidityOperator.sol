// SPDX-License-Identifier: MIT
pragma solidity >=0.8.0 <0.9.0;

/**
 * @title IClientBridgeLiquidityOperator
 * @notice 
 * Interface for the Client Bridge Liquidity Operator, responsible for
 * processing attested cross-chain bridge messages and emitting bridge intents for outbound operations. *
 * @dev
 * This contract will hold the business logic of bridge operations:
 * From the model Inbox -> MessageReceiver -> ClientBridgeLiquidityOperator
 * MessageReceivers and Inbox contracts MUST treat payloads as opaque bytes
 * and forward them unchanged to this interface.
 */
interface IClientBridgeLiquidityOperator {
     /**
     * @notice
     * Emitted when a bridge operation is initiated from this chain.
     *
     * @param sourceChainId      Originating Creditcoin chain ID
     * @param destinationChainId Target Creditcoin chain ID
     * @param destinationAddress Destination address on the target chain
     * @param msgType            Bridge message type, resolved by BridgeMessageTypeRegistry
     * @param data               Message-specific payload data
     * @param nonce              Unique intent nonce
     */
    event BridgeIntent(
        uint16 indexed sourceChainId,
        uint16 indexed destinationChainId,
        bytes32 indexed destinationAddress,
        uint8 msgType,
        bytes data,
        uint64 nonce
    );


    /**
     * @notice
     * Entry point for inbound attested messages.
     * Called by authorized MessageReceiver contracts after attestation.
     *
     * @param sourceChainId  Creditcoin chain ID of the source chain
     * @param sourceOperator Address (bytes32) of the remote bridge operator
     * @param payload        Encoded bridge message payload
     *
     * @dev
     * The payload MUST include, at minimum:
     *  - msgType (uint8), resolved by BridgeMessageTypeRegistry
     *  - destinationChainId (uint16)
     *  - destinationAddress (bytes32)
     *  - message-specific data (bytes)
     */
    function receiveBridgeMessage(
        uint16 sourceChainId,
        bytes32 sourceOperator,
        bytes calldata payload
    ) external;

    /**
     * @notice
     * Initiates an outbound bridge operation and emits a BridgeIntent event.
     *
     * @param destinationChainId Target Creditcoin chain ID
     * @param destinationAddress Destination address on the target chain
     * @param msgType            Bridge message type (protocol-level constant)
     * @param data               Message-specific payload data
     *
     * @dev
     * Implementations SHOULD validate msgType against a
     * BridgeMessageTypeRegistry before emitting the intent.
     */
    function bridgeTo(
        uint16 destinationChainId,
        bytes32 destinationAddress,
        uint8 msgType,
        bytes calldata data
    ) external;   
}