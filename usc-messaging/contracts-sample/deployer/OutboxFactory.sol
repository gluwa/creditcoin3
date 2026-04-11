// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {IOutboxFactory} from "../abstract/IOutboxFactory.sol";
import {Outbox} from "../Outbox.sol";

/// @title OutboxFactory
/// @notice Versioned factory for deploying Outbox contracts via CREATE2
/// @dev One factory = one Outbox implementation version
contract OutboxFactory is IOutboxFactory {
    function version() public pure override returns (string memory) {
        return "1.0";
    }

    function deployOutbox(
        uint32 chainKey,
        address outboxOwner,
        address validator,
        uint128 defaultRateLimit
    ) external override returns (address outbox) {
        outbox = address(
            new Outbox{salt: _salt(chainKey, msg.sender)}(
                chainKey,
                msg.sender,
                validator,
                defaultRateLimit
            )
        );

        emit OutboxCreated(outbox, chainKey, outboxOwner, validator, version());
    }

    function computeOutboxAddress(
        uint32 chainKey,
        address outboxOwner,
        address validator,
        uint128 defaultRateLimit
    ) external override view returns (address predicted) {
        bytes32 salt = _salt(chainKey, outboxOwner);

        bytes memory initCode = abi.encodePacked(
            type(Outbox).creationCode,
            abi.encode(
                address(this),
                chainKey,
                outboxOwner,
                validator,
                defaultRateLimit
            )
        );

        bytes32 hash = keccak256(
            abi.encodePacked(
                bytes1(0xff),
                address(this),
                salt,
                keccak256(initCode)
            )
        );

        predicted = address(uint160(uint256(hash)));
    }

    function _salt(
        uint32 chainKey,
        address outboxOwner
    ) internal pure returns (bytes32) {
        return keccak256(abi.encode(chainKey, outboxOwner));
    }
}
