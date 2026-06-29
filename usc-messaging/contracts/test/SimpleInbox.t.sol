// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "../src/SimpleInbox.sol";
import {IVoteValidator} from "../src/IVoteValidator.sol";

/// Minimal foundry cheatcode surface (this project has no `lib/forge-std`).
interface Vm {
    function expectRevert() external;
}

/// Vote validator that always accepts — isolates the Inbox's delivery/pending logic from
/// signature checking (which `EOAValidator.t.sol` covers).
contract AcceptValidator is IVoteValidator {
    function validateVotes(bytes32, bytes calldata) external view {}

    function attestors() external pure returns (address[] memory) {
        return new address[](0);
    }

    function threshold() external pure returns (uint256) {
        return 0;
    }
}

/// Vote validator that always rejects, to prove `deliverMessage` propagates a validation revert.
contract RejectValidator is IVoteValidator {
    function validateVotes(bytes32, bytes calldata) external pure {
        revert("bad votes");
    }

    function attestors() external pure returns (address[] memory) {
        return new address[](0);
    }

    function threshold() external pure returns (uint256) {
        return 0;
    }
}

/// Destination dApp stub. `failing` flips `receiveMessage` between revert (drives the pending path)
/// and success (drives delivery), and `calls` records successful deliveries.
contract MockDestination {
    uint256 public calls;
    bool public failing;

    function setFailing(bool f) external {
        failing = f;
    }

    function receiveMessage(bytes32, uint256, address, bytes calldata) external {
        require(!failing, "destination failing");
        calls++;
    }
}

contract SimpleInboxTest {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    SimpleInbox inbox;
    AcceptValidator acceptV;
    MockDestination dest;

    uint256 constant CC_CHAIN_ID = 42;
    bytes32 constant LOCAL_KEY = bytes32(uint256(2));
    address constant EMITTER = address(0xEEEE);

    function setUp() public {
        acceptV = new AcceptValidator();
        dest = new MockDestination();
        inbox = new SimpleInbox(address(acceptV), CC_CHAIN_ID, LOCAL_KEY);
    }

    /// Inbox payload is `abi.encode(destinationContract, innerPayload)` (see `deliverMessage`).
    function _payload(address d) internal pure returns (bytes memory) {
        return abi.encode(d, bytes("hello"));
    }

    function test_deliver_happy_path() public {
        bytes32 id = keccak256("m1");
        inbox.deliverMessage(id, EMITTER, _payload(address(dest)), "");
        require(dest.calls() == 1, "destination called once");
        require(!inbox.isPending(id), "not pending");
    }

    function test_redelivery_reverts_already_validated() public {
        bytes32 id = keccak256("m1");
        inbox.deliverMessage(id, EMITTER, _payload(address(dest)), "");
        vm.expectRevert(); // "Already validated"
        inbox.deliverMessage(id, EMITTER, _payload(address(dest)), "");
    }

    function test_invalid_votes_revert_propagates() public {
        SimpleInbox badInbox = new SimpleInbox(address(new RejectValidator()), CC_CHAIN_ID, LOCAL_KEY);
        vm.expectRevert(); // "bad votes" from the validator
        badInbox.deliverMessage(keccak256("m2"), EMITTER, _payload(address(dest)), "");
    }

    function test_pending_when_destination_reverts() public {
        dest.setFailing(true);
        bytes32 id = keccak256("m3");
        inbox.deliverMessage(id, EMITTER, _payload(address(dest)), "");
        require(inbox.isPending(id), "should be pending");
        require(dest.calls() == 0, "destination not successfully called");
    }

    function test_retry_pending_delivers_after_recovery() public {
        dest.setFailing(true);
        bytes32 id = keccak256("m4");
        inbox.deliverMessage(id, EMITTER, _payload(address(dest)), "");
        require(inbox.isPending(id), "pending");

        dest.setFailing(false);
        inbox.retryPendingMessage(id);
        require(!inbox.isPending(id), "no longer pending");
        require(dest.calls() == 1, "delivered on retry");
    }

    function test_retry_unknown_message_reverts() public {
        vm.expectRevert(); // "Not pending"
        inbox.retryPendingMessage(keccak256("never-seen"));
    }

    function test_execute_delivery_only_self() public {
        // executeDelivery is `external` but gated on `msg.sender == address(this)`.
        vm.expectRevert(); // "Only self"
        inbox.executeDelivery(address(dest), keccak256("x"), EMITTER, "");
    }
}
