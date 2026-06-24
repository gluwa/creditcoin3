// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "../src/SimpleOutbox.sol";

/// Minimal foundry cheatcode surface (this project has no `lib/forge-std`).
interface Vm {
    function prank(address) external;
    function expectRevert(bytes4) external;
    function expectRevert() external;
}

/// Covers the option-B ack gating: `acknowledgeMessage` is restricted to the configured `validator`.
contract SimpleOutboxAckTest {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    Outbox outbox;
    address constant VALIDATOR = address(0xACE);
    address constant OWNER = address(0xB0B);

    function setUp() public {
        outbox = new Outbox(bytes32(uint256(2)), VALIDATOR, OWNER);
    }

    function _publishAckMessage() internal returns (bytes32) {
        // emitter = this test contract (the calling USC); requiresAck = true.
        return outbox.publishMessage(true, "payload");
    }

    function test_non_validator_cannot_acknowledge() public {
        bytes32 id = _publishAckMessage();
        // Caller is this test contract (not VALIDATOR) -> must revert.
        vm.expectRevert(Outbox.NotValidator.selector);
        outbox.acknowledgeMessage(id);
    }

    function test_validator_can_acknowledge() public {
        bytes32 id = _publishAckMessage();
        vm.prank(VALIDATOR);
        outbox.acknowledgeMessage(id); // must not revert
        (, bool acknowledged,) = outbox.messages(id);
        require(acknowledged, "message should be acknowledged");
    }

    function test_validator_gate_precedes_other_checks() public {
        // A non-validator acking an unknown messageId still hits the validator gate first.
        vm.expectRevert(Outbox.NotValidator.selector);
        outbox.acknowledgeMessage(bytes32(uint256(0xdead)));
    }
}
