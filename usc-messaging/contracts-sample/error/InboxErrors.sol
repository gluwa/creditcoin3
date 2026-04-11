// SPDX-License-Identifier: MIT
pragma solidity >0.8.0 <0.9.0;

library InboxErrors {
    error ZeroAddress();
    error InvalidChainKey();
    error InvalidChainId();
    error ValidatorNotSet();

    error MessageAlreadyValidated(bytes32 messageId);
    error ValidationAlreadyFailed(bytes32 messageId);
    error MessageAlreadyProcessed(bytes32 messageId);
    error MessageNotPending(bytes32 messageId);
    error RetryFailed(bytes32 messageId);
}
