// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {IUSCBridgeOutbound} from "./IUSCBridgeOutbound.sol";
import {IUSCBridgeInbound} from "./IUSCBridgeInbound.sol";

/// @notice Combined USC bridge liquidity operator interface.
///         Inherit IUSCBridgeOutbound or IUSCBridgeInbound directly if only one direction is needed.
interface IUSCBridgeLiquidityOperator is IUSCBridgeOutbound, IUSCBridgeInbound {}
