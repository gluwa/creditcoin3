// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {USCBridgeTypes} from "./USCBridgeTypes.sol";

/// @title BridgeMessageCodecV1
/// @notice Canonical ABI codec for USC bridge payload bytes.
library BridgeMessageCodecV1 {
    error InvalidEvmReceiverLength(uint256 length);
    error InvalidTokenAmount();

    struct BridgePayloadV1 {
        bytes32 intentId;
        USCBridgeTypes.BridgeMessage message;
    }

    function encode(
        BridgePayloadV1 memory payload
    ) internal pure returns (bytes memory) {
        return abi.encode(payload);
    }

    function decode(
        bytes memory encodedPayload
    ) internal pure returns (BridgePayloadV1 memory) {
        return abi.decode(encodedPayload, (BridgePayloadV1));
    }

    function hasTokenTransfer(
        BridgePayloadV1 memory payload
    ) internal pure returns (bool) {
        USCBridgeTypes.EVMTokenAmount memory tokenAmount = payload
            .message
            .tokenAmount;
        if (tokenAmount.token == address(0)) {
            return false;
        }
        if (tokenAmount.amount == 0) {
            revert InvalidTokenAmount();
        }
        return true;
    }

    function hasExecutionPayload(
        BridgePayloadV1 memory payload
    ) internal pure returns (bool) {
        return payload.message.data.length > 0;
    }

    /// @notice Decodes `message.receiver` as an EVM address.
    /// @dev Receiver must be abi.encode(address), resulting in 32-byte payload.
    function decodeEvmReceiver(
        BridgePayloadV1 memory payload
    ) internal pure returns (address receiver) {
        if (payload.message.receiver.length != 32) {
            revert InvalidEvmReceiverLength(payload.message.receiver.length);
        }
        receiver = abi.decode(payload.message.receiver, (address));
    }
}
