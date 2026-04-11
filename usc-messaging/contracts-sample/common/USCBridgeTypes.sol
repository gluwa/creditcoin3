// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

library USCBridgeTypes {
    struct EVMTokenAmount {
        address token;
        uint256 amount;
    }

    /// @notice Bridge message carrying a token transfer and optional payload.
    ///         Used for both outbound (bridgeTo) and inbound (bridgeFromIntent) operations.
    struct BridgeMessage {
        /// @dev abi.encode(address) for EVM destinations; chain-specific encoding for non-EVM.
        bytes receiver;
        /// @dev Arbitrary dApp-specific payload, opaque to bridge infrastructure.
        bytes data;
        /// @dev Single token to transfer alongside the message.
        EVMTokenAmount tokenAmount;
        /// @dev Gas allocated for the destination contract call.
        uint256 gasLimit;
    }

    /// @notice Normalized attested source-transaction facts used for matching.
    struct AttestedTxData {
        address user;
        uint256 nonce;
        uint256 sourceChainId;
        EVMTokenAmount sourceAmount;
        bytes32 proofContextHash;
    }
}
