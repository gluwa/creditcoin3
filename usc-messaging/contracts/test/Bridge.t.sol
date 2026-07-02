// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {BridgeToken} from "../src/bridge/BridgeToken.sol";
import {AnvilBridge} from "../src/bridge/AnvilBridge.sol";
import {CcBridge} from "../src/bridge/CcBridge.sol";

/// Minimal foundry cheatcode surface (this project has no `lib/forge-std`).
interface Vm {
    function prank(address) external;
    function expectRevert() external;
    function expectRevert(bytes4) external;
}

/// Records the last publishMessage payload so the CC→Anvil outbound path can be asserted without a
/// real Outbox.
contract MockOutbox {
    bytes public lastPayload;
    uint256 public publishCount;

    function publishMessage(bool, bytes calldata payload) external returns (bytes32) {
        lastPayload = payload;
        publishCount++;
        return keccak256(abi.encode(publishCount, payload));
    }
}

/// Covers the escrow/release logic of both endpoints. The proof-based `CcBridge.claim` path needs
/// the live block-prover precompile and is exercised by the local 2-way e2e instead.
contract BridgeTest {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    BridgeToken anvilTok;
    BridgeToken ccTok;
    AnvilBridge anvilBridge;
    CcBridge ccBridge;
    MockOutbox outbox;

    address constant INBOX = address(0x1bb0);
    address constant USER = address(0xBEEF);

    function setUp() public {
        anvilTok = new BridgeToken("Anvil USDC", "aUSDC");
        ccTok = new BridgeToken("CC USDC", "cUSDC");
        outbox = new MockOutbox();

        anvilBridge = new AnvilBridge(address(anvilTok), INBOX);
        ccBridge = new CcBridge(address(ccTok), address(outbox), address(anvilBridge), 2);

        // Pre-fund bridge liquidity (release side) and the user (lock side).
        anvilTok.mint(address(anvilBridge), 1_000_000 ether);
        ccTok.mint(address(ccBridge), 1_000_000 ether);
        anvilTok.mint(USER, 100 ether);
        ccTok.mint(USER, 100 ether);
    }

    // --- Anvil → CC: lock side -----------------------------------------------------------------

    function test_lock_escrows_and_increments_nonce() public {
        vm.prank(USER);
        anvilTok.approve(address(anvilBridge), 30 ether);
        vm.prank(USER);
        anvilBridge.lock(30 ether, USER);

        require(anvilTok.balanceOf(address(anvilBridge)) == 1_000_030 ether, "escrowed");
        require(anvilTok.balanceOf(USER) == 70 ether, "user debited");
        require(anvilBridge.lockNonce() == 1, "nonce advanced");
    }

    // --- CC → Anvil: withdraw publishes a release message --------------------------------------

    function test_withdraw_escrows_and_publishes() public {
        vm.prank(USER);
        ccTok.approve(address(ccBridge), 25 ether);
        vm.prank(USER);
        ccBridge.withdraw(25 ether, USER);

        require(ccTok.balanceOf(address(ccBridge)) == 1_000_025 ether, "escrowed on CC");
        require(outbox.publishCount() == 1, "published once");
        // payload = abi.encode(anvilBridge, abi.encode(recipient, amount))
        (address dest, bytes memory inner) = abi.decode(outbox.lastPayload(), (address, bytes));
        require(dest == address(anvilBridge), "routed to AnvilBridge");
        (address recipient, uint256 amount) = abi.decode(inner, (address, uint256));
        require(recipient == USER && amount == 25 ether, "inner payload");
    }

    // --- CC → Anvil: receiveMessage releases on Anvil ------------------------------------------

    function test_receive_message_releases_only_via_inbox() public {
        bytes memory payload = abi.encode(USER, 10 ether);

        // Non-inbox caller is rejected.
        vm.expectRevert(AnvilBridge.NotInbox.selector);
        anvilBridge.receiveMessage(bytes32(uint256(1)), 42, address(0xabcd), payload);

        // Inbox delivery pays out.
        vm.prank(INBOX);
        anvilBridge.receiveMessage(bytes32(uint256(1)), 42, address(0xabcd), payload);
        require(anvilTok.balanceOf(USER) == 110 ether, "released to user");

        // Replaying the same messageId is rejected.
        vm.prank(INBOX);
        vm.expectRevert(AnvilBridge.AlreadyProcessed.selector);
        anvilBridge.receiveMessage(bytes32(uint256(1)), 42, address(0xabcd), payload);
    }
}
