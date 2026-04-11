// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {IBridgeMessageFormat} from "../abstract/IBridgeMessageFormat.sol";

contract BrokenBridgeMessageFormat is IBridgeMessageFormat {
    function decodeBridgePayload(
        bytes calldata
    )
        external
        pure
        override
        returns (bytes32, address, bytes memory)
    {
        revert("BrokenBridgeMessageFormat: decode failed");
    }
}
