// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @notice Minimal owner-controlled registry for indexer authorization integration tests.
///         It exposes the canonical Discovery event/getters without the production deployment stack.
contract MockOutboxDiscovery {
    address public immutable owner = msg.sender;
    mapping(uint32 => mapping(address => bool)) public isActiveOutbox;
    mapping(uint32 => address[]) private members;

    event OutboxRegistered(uint32 indexed chainKey, address indexed outbox, address indexed registrar);

    function registerOutbox(uint32 chainKey, address outbox) external {
        require(msg.sender == owner, "owner only");
        require(!isActiveOutbox[chainKey][outbox], "already active");
        isActiveOutbox[chainKey][outbox] = true;
        members[chainKey].push(outbox);
        emit OutboxRegistered(chainKey, outbox, msg.sender);
    }

    function activeOutboxes(uint32 chainKey) external view returns (address[] memory) {
        return members[chainKey];
    }
}
