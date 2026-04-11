// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {Ownable2Step} from "@openzeppelin/contracts/access/Ownable2Step.sol";
import {IOutboxFactory} from "../abstract/IOutboxFactory.sol";

/// @title OutboxDeployer
/// @notice Controls OutboxFactory versions and deploys Outboxes per chain
contract OutboxDeployer is Ownable2Step {
    error FactoryNotRegistered();
    error FactoryDisabled();
    error ChainKeyAlreadyUsed(uint32 chainKey);
    error ChainMappingExists(uint32 chainKey);
    error InvalidFactory();
    error ZeroAddress();

    event FactoryRegistered(address indexed factory, string version);
    event FactoryEnabled(address indexed factory);
    event FactoryDisabledEvent(address indexed factory);
    event ChainRegistered(uint32 indexed chainKey, uint256 indexed chainId);
    event OutboxDeployed(
        uint32 indexed chainKey,
        uint256 indexed chainId,
        address indexed outbox,
        address factory,
        string version
    );

    constructor() Ownable(msg.sender) {}

    /// @notice version string => factory address
    mapping(string => address) public factories;

    /// @notice factory address => enabled
    mapping(address => bool) public factoryEnabled;

    /// @notice chainKey => EVM chainId
    mapping(uint32 => uint256) public chainIdOf;

    /// @notice chainKey => deployed Outbox
    mapping(uint32 => address) public outboxOf;

    function registerFactory(address factory) external onlyOwner {
        string memory v = IOutboxFactory(factory).version();

        if (bytes(v).length == 0) {
            revert InvalidFactory();
        }

        factories[v] = factory;
        factoryEnabled[factory] = false;

        emit FactoryRegistered(address(factory), v);
    }

    function enableFactory(address factory) external onlyOwner {
        if (
            !factoryEnabled[factory] &&
            factories[IOutboxFactory(factory).version()] != factory
        ) {
            revert FactoryNotRegistered();
        }
        factoryEnabled[factory] = true;
        emit FactoryEnabled(factory);
    }

    function disableFactory(address factory) external onlyOwner {
        factoryEnabled[factory] = false;
        emit FactoryDisabledEvent(factory);
    }

 

    function registerChain(
        uint32 chainKey,
        uint256 chainId
    ) external onlyOwner {
        if (chainIdOf[chainKey] != 0) {
            revert ChainMappingExists(chainKey);
        }

        chainIdOf[chainKey] = chainId;
        emit ChainRegistered(chainKey, chainId);
    }


    function deployOutbox(
        string calldata version,
        uint32 chainKey,
        address outboxOwner,
        address validator,
        uint128 defaultRateLimit
    ) external onlyOwner returns (address outbox) {
        address factory = factories[version];
        if (factory == address(0)) revert FactoryNotRegistered();
        if (!factoryEnabled[factory]) revert FactoryDisabled();
        if (outboxOf[chainKey] != address(0))
            revert ChainKeyAlreadyUsed(chainKey);
        if (chainIdOf[chainKey] == 0) revert ChainMappingExists(chainKey);

        outbox = IOutboxFactory(factory).deployOutbox(
            chainKey,
            outboxOwner,
            validator,
            defaultRateLimit
        );

        outboxOf[chainKey] = outbox;

        emit OutboxDeployed(
            chainKey,
            chainIdOf[chainKey],
            outbox,
            factory,
            version
        );
    }
}
