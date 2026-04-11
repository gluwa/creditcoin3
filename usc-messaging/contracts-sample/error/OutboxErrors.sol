// SPDX-License-Identifier: MIT
pragma solidity >0.8.0 <0.9.0;

library OutboxErrors {
    error NotValidator();
    error ZeroAddress();
    error AlreadyInitialized();

    error MessageNotFound(bytes32 messageId);
    error MessageDoesNotRequireAck(bytes32 messageId);
    error MessageAlreadyAcknowledged(bytes32 messageId);
}
