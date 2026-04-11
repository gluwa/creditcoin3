// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {USCBridgeTypes} from "../common/USCBridgeTypes.sol";

/// @notice Outbound bridge interface: Creditcoin → client chain.
interface IUSCBridgeOutbound {
    /// @notice Emitted by bridgeTo when an outbound bridge operation is initiated.
    /// @param intentId keccak256(abi.encodePacked(chainKey, baseIntentId)).
    /// @param chainKey Chain key of the destination client chain.
    /// @param message  Full outbound message.
    event BridgeIntent(
        bytes32 indexed intentId,
        bytes32 indexed chainKey,
        USCBridgeTypes.BridgeMessage message
    );

    /// @notice Initiates a bridge operation from Creditcoin to a client chain.
    ///         Emits BridgeIntent. Tokens in message.tokenAmount are pulled from msg.sender via transferFrom.
    /// @param chainKey Chain key of the destination client chain (must be whitelisted).
    /// @param message  Outbound message describing receiver, payload, token, and gas limit.
    /// @param quote    ABI-encoded signed quote from the off-chain quoter for relay fee payment.
    function bridgeTo(
        bytes32 chainKey,
        USCBridgeTypes.BridgeMessage calldata message,
        bytes calldata quote
    ) external payable;
}
