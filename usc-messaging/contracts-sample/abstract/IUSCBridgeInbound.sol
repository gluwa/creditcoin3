// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {BlockProverTypes} from "../common/BlockProverTypes.sol";
import {CrossChainOrderTypes} from "../common/CrossChainOrderTypes.sol";
import {USCBridgeTypes} from "../common/USCBridgeTypes.sol";

/// @notice Inbound bridge interface: client chain → Creditcoin (ERC-7683).
interface IUSCBridgeInbound {
    /// @notice Emitted when an inbound ERC-7683 cross-chain order from a client chain is successfully processed on Creditcoin.
    /// @param intentId Identifier derived from the client-chain order (keccak256 of chainKey + nonce + user).
    /// @param chainKey Chain key of the source client chain.
    /// @param order    The original ERC-7683 order decoded from the client-chain log.
    event CrossChainOrderProcessed(
        bytes32 indexed intentId,
        bytes32 indexed chainKey,
        CrossChainOrderTypes.CrossChainOrder order
    );

    /// @notice Processes all CrossChainOrderTypes.CrossChainOrder events in a client-chain transaction.
    ///         Decodes encodedTransaction logs by event topic into CrossChainOrder structs,
    ///         then processes each order. Reverts if any matching order was already processed.
    /// @param chainKey            Chain key of the source client chain.
    /// @param blockHeight         Block height containing the transaction.
    /// @param encodedTransaction  RLP-encoded transaction + receipt whose logs are decoded into CrossChainOrder.
    /// @param inclusionProof      Self-describing proof that the transaction is in the block.
    /// @param continuityProof     Chain-continuity proof preventing re-org attacks.
    function bridgeFromIntent(
        bytes32 chainKey,
        uint64 blockHeight,
        bytes calldata encodedTransaction,
        BlockProverTypes.InclusionProof calldata inclusionProof,
        BlockProverTypes.ContinuityProof calldata continuityProof
    )
        external
        returns (bool isValid, bytes[] memory extractedTransactionData);
}
