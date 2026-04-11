// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @title IOutboxFactory
/// @notice Minimalinterface for OutboxFactory contracts, all future OutboxFactory contracts must implement this (like OutboxFactoryV2, etc)
interface IOutboxFactory {
    /// @notice The factory version (e.g. "1.0")
    function version() external view returns (string memory);

    /// @notice Emitted when a new Outbox contract is deployed
    /// @param outbox Deployed Outbox address
    /// @param chainKey Logical client chain key
    /// @param owner Outbox owner
    /// @param validator Validator address
    /// @param version Factory version string
    event OutboxCreated(
        address indexed outbox,
        uint32 indexed chainKey,
        address indexed owner,
        address validator,
        string version
    );

    /// @notice Deploys an Outbox instance for a client chain via CREATE2
    /// @param chainKey Client chain identifier match to the ChainId of target network
    /// @param outboxOwner Owner of the deployed Outbox
    /// @param validator Validator contract address
    /// @param defaultRateLimit Default rate limit config
    function deployOutbox(
        uint32 chainKey,
        address outboxOwner,
        address validator,
        uint128 defaultRateLimit
    ) external returns (address outbox);

    function computeOutboxAddress(
        uint32 chainId,
        address outboxOwner,
        address validator,
        uint128 defaultRateLimit
    ) external view returns (address predicted);
}
