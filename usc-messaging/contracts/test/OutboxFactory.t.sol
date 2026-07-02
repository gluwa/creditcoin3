// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

// Brings in `OutboxFactory`, the `IOutboxFactory` interface, and the concrete `Outbox`.
import "../src/SimpleOutboxFactory.sol";

/// Minimal foundry cheatcode surface (this project has no `lib/forge-std`).
interface Vm {
    function prank(address) external;
    function expectRevert() external;
}

contract OutboxFactoryTest {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    OutboxFactory factory;
    bytes32 constant CHAIN_KEY = bytes32(uint256(2));
    address constant VALIDATOR = address(0xACE);
    address constant STRANGER = address(0xBAD);

    function setUp() public {
        // Deployer (this test contract) becomes the factory owner.
        factory = new OutboxFactory();
    }

    function test_create_outbox_wires_fields() public {
        address ob = factory.createOutbox(CHAIN_KEY, VALIDATOR);
        require(ob != address(0), "outbox created");
        require(factory.getOutbox(CHAIN_KEY) == ob, "registered under chainKey");

        Outbox o = Outbox(ob);
        require(o.chainKey() == CHAIN_KEY, "chainKey passed through");
        require(o.validator() == VALIDATOR, "validator passed through");
        // The factory hands the outbox its own owner, so the same account controls both.
        require(o.owner() == address(this), "outbox owner is factory owner");
    }

    function test_create_outbox_only_owner() public {
        vm.prank(STRANGER);
        vm.expectRevert(); // "Not authorized"
        factory.createOutbox(CHAIN_KEY, VALIDATOR);
    }

    function test_duplicate_chain_key_reverts() public {
        factory.createOutbox(CHAIN_KEY, VALIDATOR);
        vm.expectRevert(); // "Outbox already exists"
        factory.createOutbox(CHAIN_KEY, address(0xBEE));
    }

    function test_zero_validator_reverts() public {
        vm.expectRevert(); // "Invalid validator"
        factory.createOutbox(CHAIN_KEY, address(0));
    }

    function test_get_unknown_outbox_is_zero() public view {
        require(factory.getOutbox(bytes32(uint256(99))) == address(0), "unknown chainKey is zero");
    }
}
