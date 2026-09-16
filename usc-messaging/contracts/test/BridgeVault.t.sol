// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {Test} from "forge-std/Test.sol";
import {BridgeVault} from "../src/BridgeVault.sol";
import {MockERC20} from "./mocks/MockERC20.sol";

contract BridgeVaultTest is Test {
    BridgeVault internal vault;
    MockERC20 internal token;

    address internal owner = makeAddr("owner");
    address internal inbox = makeAddr("inbox");
    address internal bridgeHub = makeAddr("bridgeHub");
    address internal depositor = makeAddr("depositor");
    address internal recipient = makeAddr("recipient");

    uint32 internal constant DEST_CHAIN_KEY = 8;

    function setUp() public {
        vault = new BridgeVault(inbox, owner, bridgeHub);
        token = new MockERC20();
    }

    // ---------------------------------------------------------------------
    // deposit — native ETH
    // ---------------------------------------------------------------------

    function test_deposit_native_locksFundsAndEmits() public {
        vm.deal(depositor, 1 ether);

        vm.expectEmit(true, true, true, true, address(vault));
        emit BridgeVault.Deposited(0, depositor, recipient, DEST_CHAIN_KEY, address(0), 1 ether);

        vm.prank(depositor);
        vault.deposit{value: 1 ether}(address(0), 1 ether, DEST_CHAIN_KEY, recipient);

        assertEq(address(vault).balance, 1 ether);
        assertEq(vault.depositNonce(), 1);
    }

    function test_deposit_native_incrementsNonceAcrossCalls() public {
        vm.deal(depositor, 2 ether);
        vm.startPrank(depositor);
        vault.deposit{value: 1 ether}(address(0), 1 ether, DEST_CHAIN_KEY, recipient);
        vault.deposit{value: 1 ether}(address(0), 1 ether, DEST_CHAIN_KEY, recipient);
        vm.stopPrank();

        assertEq(vault.depositNonce(), 2);
    }

    function test_deposit_revertsOnZeroAmount() public {
        vm.expectRevert(BridgeVault.ZeroAmount.selector);
        vault.deposit(address(0), 0, DEST_CHAIN_KEY, recipient);
    }

    function test_deposit_revertsOnZeroRecipient() public {
        vm.deal(depositor, 1 ether);
        vm.prank(depositor);
        vm.expectRevert(BridgeVault.ZeroRecipient.selector);
        vault.deposit{value: 1 ether}(address(0), 1 ether, DEST_CHAIN_KEY, address(0));
    }

    function test_deposit_revertsOnNativeAmountMismatch() public {
        vm.deal(depositor, 1 ether);
        vm.prank(depositor);
        vm.expectRevert(
            abi.encodeWithSelector(BridgeVault.NativeAmountMismatch.selector, 1 ether, 0.5 ether)
        );
        vault.deposit{value: 0.5 ether}(address(0), 1 ether, DEST_CHAIN_KEY, recipient);
    }

    // ---------------------------------------------------------------------
    // deposit — ERC-20
    // ---------------------------------------------------------------------

    function test_deposit_erc20_revertsWhenNotAllowed() public {
        token.mint(depositor, 100 ether);
        vm.prank(depositor);
        vm.expectRevert(
            abi.encodeWithSelector(BridgeVault.UnsupportedToken.selector, address(token))
        );
        vault.deposit(address(token), 100 ether, DEST_CHAIN_KEY, recipient);
    }

    function test_deposit_erc20_locksFundsWhenAllowed() public {
        vm.prank(owner);
        vault.setAllowedToken(address(token), true);

        token.mint(depositor, 100 ether);
        vm.startPrank(depositor);
        token.approve(address(vault), 100 ether);
        vault.deposit(address(token), 100 ether, DEST_CHAIN_KEY, recipient);
        vm.stopPrank();

        assertEq(token.balanceOf(address(vault)), 100 ether);
    }

    function test_deposit_erc20_revertsIfNativeValueSent() public {
        vm.prank(owner);
        vault.setAllowedToken(address(token), true);

        vm.deal(depositor, 1 ether);
        token.mint(depositor, 100 ether);
        vm.startPrank(depositor);
        token.approve(address(vault), 100 ether);
        vm.expectRevert(
            abi.encodeWithSelector(BridgeVault.NativeAmountMismatch.selector, 0, 1 ether)
        );
        vault.deposit{value: 1 ether}(address(token), 100 ether, DEST_CHAIN_KEY, recipient);
        vm.stopPrank();
    }

    // ---------------------------------------------------------------------
    // release (releaseFromBridge, called the way DispatcherRouter's
    // DefaultDispatcher/DestinationCall actually calls it: a plain external call whose
    // calldata is `abi.encodeWithSelector(releaseFromBridge, ...) ++ bytes20(emitter)` —
    // see asc-contracts write-ability/common/DestinationCall.sol. Built by hand here (not
    // via a typed BridgeVault.releaseFromBridge(...) call) so these tests exercise the exact
    // trailing-emitter extraction production traffic relies on, not just a shape that happens
    // to satisfy Solidity's own call syntax.
    // ---------------------------------------------------------------------

    function _releaseCalldata(
        bytes32 messageId,
        address to,
        address releaseToken,
        uint256 amount,
        address emitter
    ) internal pure returns (bytes memory) {
        bytes memory core = abi.encodeWithSelector(
            BridgeVault.releaseFromBridge.selector, messageId, to, releaseToken, amount
        );
        return bytes.concat(core, bytes20(emitter));
    }

    function test_release_native_paysRecipientAndEmits() public {
        vm.deal(address(vault), 1 ether);
        bytes32 messageId = keccak256("msg-1");
        bytes memory data = _releaseCalldata(messageId, recipient, address(0), 1 ether, bridgeHub);

        vm.expectEmit(true, true, true, true, address(vault));
        emit BridgeVault.Released(messageId, recipient, address(0), 1 ether);

        vm.prank(inbox);
        (bool ok,) = address(vault).call(data);
        assertTrue(ok);

        assertEq(recipient.balance, 1 ether);
    }

    function test_release_revertsOnUntrustedEmitter() public {
        vm.deal(address(vault), 1 ether);
        address rogue = makeAddr("rogue");
        bytes memory data =
            _releaseCalldata(keccak256("msg-2"), recipient, address(0), 1 ether, rogue);

        vm.prank(inbox);
        vm.expectRevert(
            abi.encodeWithSelector(BridgeVault.UntrustedEmitter.selector, rogue, bridgeHub)
        );
        address(vault).call(data);
    }

    function test_release_revertsWhenCallerIsNotTrustedInbox() public {
        address notInbox = makeAddr("notInbox");
        bytes memory data =
            _releaseCalldata(keccak256("msg-3"), recipient, address(0), 1 ether, bridgeHub);

        vm.prank(notInbox);
        vm.expectRevert(abi.encodeWithSignature("UnauthorizedInbox(address)", notInbox));
        address(vault).call(data);
    }

    function test_release_revertsOnInsufficientLiquidity_thenSucceedsAfterTopUp() public {
        bytes32 messageId = keccak256("msg-4");
        bytes memory data = _releaseCalldata(messageId, recipient, address(0), 1 ether, bridgeHub);

        // Vault has no balance yet — the release must revert, and (mirroring the old
        // MessageReceiverBase-based design) that revert also rolls back the "processed" write for
        // this messageId, since `processedMessages[messageId] = true` is written before `_release`
        // is called but a revert unwinds the whole call, including that storage write.
        vm.prank(inbox);
        vm.expectRevert(
            abi.encodeWithSelector(
                BridgeVault.InsufficientLiquidity.selector, address(0), 1 ether, 0
            )
        );
        address(vault).call(data);

        // Top up, then retry the *same* messageId — must succeed, proving no partial state stuck.
        vm.deal(address(vault), 1 ether);
        vm.prank(inbox);
        (bool ok,) = address(vault).call(data);
        assertTrue(ok);

        assertEq(recipient.balance, 1 ether);
    }

    function test_release_revertsOnReplay() public {
        vm.deal(address(vault), 2 ether);
        bytes32 messageId = keccak256("msg-5");
        bytes memory data = _releaseCalldata(messageId, recipient, address(0), 1 ether, bridgeHub);

        vm.prank(inbox);
        (bool ok,) = address(vault).call(data);
        assertTrue(ok);

        vm.prank(inbox);
        vm.expectRevert(abi.encodeWithSignature("MessageAlreadyProcessed(bytes32)", messageId));
        address(vault).call(data);
    }

    // ---------------------------------------------------------------------
    // release, ERC-20
    // ---------------------------------------------------------------------

    function test_release_erc20_paysRecipient() public {
        token.mint(address(vault), 50 ether);
        bytes32 messageId = keccak256("msg-erc20");
        bytes memory data =
            _releaseCalldata(messageId, recipient, address(token), 50 ether, bridgeHub);

        vm.prank(inbox);
        (bool ok,) = address(vault).call(data);
        assertTrue(ok);

        assertEq(token.balanceOf(recipient), 50 ether);
    }

    // ---------------------------------------------------------------------
    // owner administration
    // ---------------------------------------------------------------------

    function test_setBridgeHub_onlyOwner() public {
        address notOwner = makeAddr("notOwner");
        vm.prank(notOwner);
        vm.expectRevert(abi.encodeWithSignature("OwnableUnauthorizedAccount(address)", notOwner));
        vault.setBridgeHub(makeAddr("newHub"));
    }

    function test_setBridgeHub_updatesTrustedEmitter() public {
        address newHub = makeAddr("newHub");
        vm.prank(owner);
        vault.setBridgeHub(newHub);
        assertEq(vault.bridgeHub(), newHub);
    }
}
