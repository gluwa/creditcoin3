// SPDX-License-Identifier: MIT
pragma solidity >0.8.0 <0.9.0;

library ClientBridgeLiquidityOperatorErrors {
    error ZeroAddress();
    error InvalidChainId(uint256 chainId);
    error UnauthorizedChain(uint16 chainId);
    error UnauthorizedEmitter(address emitter);
    error IntentAlreadyProcessed(bytes32 intentId);
    error NonceAlreadyUsed(address user, uint256 nonce);
    error FunctionNotImplemented();
    error EmptyDestinationAddress();
}