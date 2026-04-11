// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {IBridgeMessageFormat} from "../abstract/IBridgeMessageFormat.sol";
import {BridgeMessageCodecV1} from "./BridgeMessageCodecV1.sol";

contract BridgeMessageFormatV1 is IBridgeMessageFormat {
    error EmptyExecutionPayload();

    function decodeBridgePayload(
        bytes calldata payload
    ) external pure override returns (bytes32 intentId, address destinationAddress, bytes memory callData) {
        BridgeMessageCodecV1.BridgePayloadV1 memory bridgePayload = BridgeMessageCodecV1
            .decode(payload);

        if (!BridgeMessageCodecV1.hasExecutionPayload(bridgePayload)) {
            revert EmptyExecutionPayload();
        }

        intentId = bridgePayload.intentId;
        destinationAddress = BridgeMessageCodecV1.decodeEvmReceiver(
            bridgePayload
        );
        callData = bridgePayload.message.data;
    }
}
