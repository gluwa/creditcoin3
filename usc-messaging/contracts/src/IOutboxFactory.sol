// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

interface IOutboxFactory {
    /// @notice Creates a new outbox contract for a chain
    /// @param chainKey The identifier for the target chain
    /// @param validator The validator contract address that will call acknowledgeMessage on the outbox
    /// @return outboxAddress The address of the created outbox contract
    /// @dev The outbox contract will use the same owner as the factory
    function createOutbox(bytes32 chainKey, address validator) external returns (address outboxAddress);

    /// @notice Gets the outbox address for a given chain
    /// @param chainKey The identifier for the target chain
    /// @return outboxAddress The address of the outbox contract, or address(0) if not created
    function getOutbox(bytes32 chainKey) external view returns (address outboxAddress);

    /// @notice Emitted when a new outbox is created
    event OutboxCreated(bytes32 indexed chainKey, address indexed outboxAddress);
}
