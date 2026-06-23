// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "./IOutboxFactory.sol";
// The concrete `Outbox` contract instantiated below lives in SimpleOutbox.sol.
import "./SimpleOutbox.sol";

/// @notice Creates and manages one Outbox per chain. The factory must be operated by USC (not
/// publicly callable): only the owner can create outboxes and set their validators. The factory
/// passes its own owner to each outbox so the same account controls the factory and every outbox.
contract OutboxFactory is IOutboxFactory {
    mapping(bytes32 => address) public outboxes;
    address public owner;

    modifier onlyOwner() {
        require(msg.sender == owner, "Not authorized");
        _;
    }

    constructor() {
        owner = msg.sender;
    }

    function createOutbox(bytes32 chainKey, address validator)
        external
        override
        onlyOwner
        returns (address)
    {
        require(outboxes[chainKey] == address(0), "Outbox already exists");
        require(validator != address(0), "Invalid validator");

        // Pass the factory's owner and chainKey to the outbox contract.
        Outbox outbox = new Outbox(chainKey, validator, owner);
        address outboxAddress = address(outbox);
        outboxes[chainKey] = outboxAddress;

        emit OutboxCreated(chainKey, outboxAddress);
        return outboxAddress;
    }

    function getOutbox(bytes32 chainKey) external view override returns (address) {
        return outboxes[chainKey];
    }
}
