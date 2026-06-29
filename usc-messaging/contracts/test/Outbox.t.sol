// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "../src/SimpleOutbox.sol";

/// Minimal foundry cheatcode surface (this project has no `lib/forge-std`).
interface Vm {
    function prank(address) external;
    // `bytes` overload: these Outbox errors carry a `bytes32 messageId`, so the revert data is
    // selector ++ arg (not the bare 4-byte selector that `expectRevert(bytes4)` matches).
    function expectRevert(bytes calldata) external;
}

/// Covers `publishMessage` (messageId derivation, sequence, requiresAck) and the full
/// `acknowledgeMessage` / `batchAcknowledgeMessages` paths. The validator-gate revert is covered
/// separately in `SimpleOutboxAck.t.sol`.
contract OutboxTest {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    Outbox outbox;
    address constant VALIDATOR = address(0xACE);
    address constant OWNER = address(0xB0B);

    function setUp() public {
        outbox = new Outbox(bytes32(uint256(2)), VALIDATOR, OWNER);
    }

    function test_publish_stores_message() public {
        bytes32 id = outbox.publishMessage(true, "payload");
        (address emitter, bool acknowledged, bytes32 payloadHash) = outbox.messages(id);
        require(emitter == address(this), "emitter is the calling USC");
        require(!acknowledged, "not acknowledged at publish");
        require(payloadHash == keccak256(bytes("payload")), "payloadHash recorded");
        require(outbox.messageRequiresAck(id), "requiresAck flag stored");
    }

    function test_message_id_derivation_and_sequence() public {
        // Sequence is per-emitter and increments, so repeated identical payloads get distinct ids.
        bytes32 id1 = outbox.publishMessage(false, "a");
        bytes32 id2 = outbox.publishMessage(false, "a");
        require(id1 != id2, "ids differ by sequence");
        require(outbox.uscSequences(address(this)) == 2, "sequence advanced to 2");

        // First publish used seq 1; recompute it the same way the contract does.
        bytes32 expected1 = keccak256(abi.encode(address(outbox), address(this), uint64(1), keccak256(bytes("a"))));
        require(id1 == expected1, "messageId derivation matches keccak(outbox,usc,seq,payloadHash)");
    }

    function test_acknowledge_happy_path() public {
        bytes32 id = outbox.publishMessage(true, "payload");
        vm.prank(VALIDATOR);
        outbox.acknowledgeMessage(id);
        (, bool acknowledged,) = outbox.messages(id);
        require(acknowledged, "acknowledged");
    }

    function test_acknowledge_unknown_reverts() public {
        bytes32 unknown = bytes32(uint256(0xdead));
        vm.prank(VALIDATOR);
        vm.expectRevert(abi.encodeWithSelector(Outbox.MessageNotFound.selector, unknown));
        outbox.acknowledgeMessage(unknown);
    }

    function test_acknowledge_twice_reverts() public {
        bytes32 id = outbox.publishMessage(true, "payload");
        vm.prank(VALIDATOR);
        outbox.acknowledgeMessage(id);
        vm.prank(VALIDATOR);
        vm.expectRevert(abi.encodeWithSelector(Outbox.MessageAlreadyAcknowledged.selector, id));
        outbox.acknowledgeMessage(id);
    }

    function test_acknowledge_non_ack_message_reverts() public {
        bytes32 id = outbox.publishMessage(false, "payload"); // requiresAck = false
        vm.prank(VALIDATOR);
        vm.expectRevert(abi.encodeWithSelector(Outbox.MessageDoesNotRequireAck.selector, id));
        outbox.acknowledgeMessage(id);
    }

    function test_batch_acknowledge() public {
        bytes32 id1 = outbox.publishMessage(true, "p1");
        bytes32 id2 = outbox.publishMessage(true, "p2");
        bytes32[] memory ids = new bytes32[](2);
        ids[0] = id1;
        ids[1] = id2;

        // One pranked call covers the whole batch; internal acknowledgeMessage calls keep msg.sender.
        vm.prank(VALIDATOR);
        outbox.batchAcknowledgeMessages(ids);

        (, bool a1,) = outbox.messages(id1);
        (, bool a2,) = outbox.messages(id2);
        require(a1 && a2, "both messages acknowledged");
    }
}
