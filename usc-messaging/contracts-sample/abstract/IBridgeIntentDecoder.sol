// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {CrossChainOrderTypes} from "../common/CrossChainOrderTypes.sol";
import {USCBridgeTypes} from "../common/USCBridgeTypes.sol";

interface IBridgeIntentDecoder {

    /// @notice Decoder output used by USC bridge inbound processing.
    struct DecodedBridgeIntent {
        CrossChainOrderTypes.CrossChainOrder order;
        USCBridgeTypes.AttestedTxData attestedTxData;
    }

    /// @notice Decodes source-chain tx/receipt bytes into a single inbound ERC-7683 order.
    function decodeBridgeIntent(
        bytes calldata encodedTransaction
    ) external view returns (DecodedBridgeIntent memory);
}
