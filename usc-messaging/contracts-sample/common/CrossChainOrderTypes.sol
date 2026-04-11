// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {USCBridgeTypes} from "./USCBridgeTypes.sol";

library CrossChainOrderTypes {
    /// @notice ERC-7683 gasless cross-chain order submitted by a user.
    /// @dev orderData encodes a CrossChainIntent struct.
    struct CrossChainOrder {
        address originSettler;
        address user;
        uint256 nonce;
        uint256 originChainId;
        uint32 openDeadline;
        uint32 fillDeadline;
        bytes orderData;
    }

    /// @notice used for multiple intents in the same order, e.g. for multi-leg swaps or batch transactions.
    struct BatchCrossChainOrder {
        /// @dev Address of the settlement contract on the origin chain responsible for processing this order.
        address settlementContract;
        /// @dev Address of the user who submitted the order.
        address user;
        /// @dev Per-user nonce to prevent replay attacks.
        uint256 nonce;
        /// @dev Chain ID of the origin chain where the order was submitted.
        uint32 originChainId;
        /// @dev Timestamp by which the order must be initiated; expired orders must be rejected.
        uint32 initiateDeadline;
        /// @dev Timestamp by which the order must be filled on the destination chain.
        uint32 fillDeadline;
        /// @dev Tokens pulled from the user on the origin chain to fund the order.
        USCBridgeTypes.EVMTokenAmount[] userInputs;
        /// @dev Tokens the user expects to receive on the destination chain.
        TokenOutput[] userOutputs;
        /// @dev Tokens awarded to the filler on the origin chain as a relay incentive.
        TokenOutput[] fillerOutputs;
    }

    /// @notice Specifies a token amount to be delivered to a recipient on a target chain.
    struct TokenOutput {
        /// @dev ERC-20 token and amount; use address(0) for native token.
        USCBridgeTypes.EVMTokenAmount tokenAmount;
        /// @dev Address to receive the token output on the target chain.
        address recipient;
        /// @dev Chain ID of the destination chain for this output.
        uint32 chainId;
    }

    /// @notice High-level action requested by the order.
    enum OrderAction {
        MINT,
        TRANSFER,
        SWAP,
        CALL
    }

    /// @notice Application-level intent payload encoded into CrossChainOrder.orderData.
    ///
    /// ## Intent signing
    ///
    /// Before submitting a CrossChainOrder the user must sign the intent hash so
    /// that the settler can verify authenticity without an on-chain token approval from
    /// the user.
    ///
    /// ### Hash construction
    ///
    /// ```solidity
    /// bytes32 intentHash = keccak256(
    ///     abi.encode(
    ///         action,
    ///         sourceChainId,
    ///         destinationChainId,
    ///         sourceAmount.token,
    ///         sourceAmount.amount,
    ///         minDestinationAmount.token,
    ///         minDestinationAmount.amount,
    ///         recipient,
    ///         destinationContract,
    ///         destinationCallData,
    ///         maxGasCost,
    ///         nonce,      // CrossChainOrder.nonce
    ///         deadline    // CrossChainOrder.fillDeadline
    ///     )
    /// );
    /// ```
    ///
    /// `nonce` and `deadline` come from the enclosing `CrossChainOrder`; all
    /// other fields come from this struct.
    ///
    /// The resulting `signature` is passed to the settler alongside the encoded
    /// `CrossChainOrder`. The settler recovers the signer with
    /// `ECDSA.recover(intentHash, signature)` and checks it equals `order.user`.
    ///
    /// ### Security notes
    ///
    /// - `nonce` must be tracked per-user on the settler to prevent replay attacks.
    /// - `fillDeadline` is used as `deadline`; orders received after this timestamp
    ///   must be rejected.
    /// - `sourceProofRequirement` is intentionally excluded from the hash; it is enforced by the settler independently and must not be spoofable by the filler.
    /// @dev Decode from CrossChainOrder.orderData via: abi.decode(order.orderData, (CrossChainIntent))
    struct CrossChainIntent {
        OrderAction action;
        uint256 sourceChainId;
        uint256 destinationChainId;
        /// @dev Source token and the exact amount the user is sending.
        USCBridgeTypes.EVMTokenAmount sourceAmount;
        /// @dev Destination token and the minimum amount the recipient must receive.
        USCBridgeTypes.EVMTokenAmount minDestinationAmount;
        address recipient;
        address destinationContract;
        bytes destinationCallData;
        uint256 maxGasCost;
        bytes32 sourceProofRequirement;
    }
}
