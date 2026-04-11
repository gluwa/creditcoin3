// SPDX-License-Identifier: MIT
pragma solidity >0.8.0 <0.9.0;

library USCBridgeLiquidityOperatorErrors {
    error ZeroAddress();
    error EmptyReceiver();
    error EmptyBridgeMessage();
    error InvalidTokenAmount();
    error InvalidTokenAddress(address token);
    error ChainKeyNotConfigured(bytes32 chainKey);
    error ChainKeyDisabled(bytes32 chainKey);
    error QuoteValidationFailed();
    error ProofValidationFailed();
    error MissingAdapterConfig();
    error IntentAlreadyProcessed(bytes32 intentId);
    error InvalidIntentOrderData();
    error UnsupportedIntentAction(uint8 action);
    error IntentProofRequirementMismatch();
    error AttestedUserMismatch(address expected, address actual);
    error AttestedNonceMismatch(uint256 expected, uint256 actual);
    error AttestedSourceChainMismatch(uint256 expected, uint256 actual);
    error AttestedSourceTokenMismatch(address expected, address actual);
    error AttestedSourceAmountMismatch(uint256 expected, uint256 actual);
    error MaxGasCostExceeded(uint256 maxGasCost, uint256 actualGasCost);
    error InvalidBlockHeight();
    error EmptyEncodedTransaction();
}
