// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

interface IBridgeMessageFormat {
    function decodeBridgePayload(
        bytes calldata payload
    ) external pure returns (bytes32 intentId, address destinationAddress, bytes memory callData);
}
