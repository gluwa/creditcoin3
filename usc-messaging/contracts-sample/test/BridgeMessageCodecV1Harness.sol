// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {BridgeMessageCodecV1} from "../common/BridgeMessageCodecV1.sol";

contract BridgeMessageCodecV1Harness {
    function decodeEvmReceiver(
        BridgeMessageCodecV1.BridgePayloadV1 calldata payload
    ) external pure returns (address) {
        return BridgeMessageCodecV1.decodeEvmReceiver(payload);
    }

    function hasTokenTransfer(
        BridgeMessageCodecV1.BridgePayloadV1 calldata payload
    ) external pure returns (bool) {
        return BridgeMessageCodecV1.hasTokenTransfer(payload);
    }

    function hasExecutionPayload(
        BridgeMessageCodecV1.BridgePayloadV1 calldata payload
    ) external pure returns (bool) {
        return BridgeMessageCodecV1.hasExecutionPayload(payload);
    }

    function encodePayload(
        BridgeMessageCodecV1.BridgePayloadV1 calldata payload
    ) external pure returns (bytes memory) {
        return BridgeMessageCodecV1.encode(payload);
    }

    function decodePayload(
        bytes calldata encodedPayload
    ) external pure returns (BridgeMessageCodecV1.BridgePayloadV1 memory) {
        return BridgeMessageCodecV1.decode(encodedPayload);
    }
}
