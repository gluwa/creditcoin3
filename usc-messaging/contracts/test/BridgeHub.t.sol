// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {Test, Vm} from "forge-std/Test.sol";
import {BridgeHub, IBridgeVaultRelease} from "../src/BridgeHub.sol";
import {BridgeVault} from "../src/BridgeVault.sol";
import {BlockProverTypes} from "@gluwa/usc-contracts/write-ability/common/BlockProverTypes.sol";
import {EvmV1Decoder} from "@gluwa/usc-contracts/common/EvmV1Decoder.sol";
import {EVMPayloadCodec} from "@gluwa/usc-contracts/write-ability/common/EVMPayloadCodec.sol";
import {MockASCProofVerifier} from "./mocks/MockASCProofVerifier.sol";
import {MockOutbox} from "./mocks/MockOutbox.sol";
import {MockERC20} from "./mocks/MockERC20.sol";

contract BridgeHubTest is Test {
    BridgeHub internal hub;
    MockASCProofVerifier internal verifier;
    MockERC20 internal attestToken;

    BridgeVault internal sourceVault; // plays the source-chain BridgeVault
    MockOutbox internal destOutbox; // stands in for the destination chain's Outbox

    address internal owner = makeAddr("owner");
    address internal depositor = makeAddr("depositor");
    address internal recipient = makeAddr("recipient");
    address internal inboxStub = makeAddr("inboxStub");
    address internal bridgeHubStub = makeAddr("bridgeHubStub");
    address internal destVaultPlaceholder = makeAddr("destVaultPlaceholder");

    uint32 internal constant SRC_CHAIN_KEY = 8;
    uint32 internal constant DEST_CHAIN_KEY = 3;
    /// @dev Must match BridgeHub's private RELEASE_GAS_LIMIT constant.
    uint256 internal constant RELEASE_GAS_LIMIT = 200_000;

    function setUp() public {
        verifier = new MockASCProofVerifier();
        attestToken = new MockERC20();
        hub = new BridgeHub(address(verifier), address(attestToken), owner);

        // sourceVault only needs to exist so it can emit a real Deposited log for us to capture —
        // its own bridgeHub/inbox wiring is irrelevant to BridgeHub.claim's tests.
        sourceVault = new BridgeVault(inboxStub, owner, bridgeHubStub);
        destOutbox = new MockOutbox();

        vm.startPrank(owner);
        hub.setChainConfig(SRC_CHAIN_KEY, address(sourceVault), address(destOutbox), true);
        hub.setChainConfig(DEST_CHAIN_KEY, destVaultPlaceholder, address(destOutbox), true);
        vm.stopPrank();
    }

    /// @dev Performs a real deposit so the emitted `Deposited` log is byte-for-byte what
    ///      BridgeVault actually produces, then packs it into the `abi.encode(uint8, bytes[])`
    ///      shape EvmV1Decoder expects out of a proven transaction's receipt chunk.
    function _depositAndBuildEncodedTx(uint256 amount) internal returns (bytes memory encodedTx) {
        vm.deal(depositor, amount);
        vm.recordLogs();
        vm.prank(depositor);
        sourceVault.deposit{value: amount}(address(0), amount, DEST_CHAIN_KEY, recipient);
        Vm.Log[] memory logs = vm.getRecordedLogs();

        Vm.Log memory depositedLog;
        bool found;
        for (uint256 i; i < logs.length; ++i) {
            if (
                logs[i].emitter == address(sourceVault)
                    && logs[i].topics[0] == BridgeVault.Deposited.selector
            ) {
                depositedLog = logs[i];
                found = true;
                break;
            }
        }
        require(found, "test: Deposited log not found");

        EvmV1Decoder.LogEntryTuple[] memory decodedLogs = new EvmV1Decoder.LogEntryTuple[](1);
        decodedLogs[0] = EvmV1Decoder.LogEntryTuple({
            address_: depositedLog.emitter, topics: depositedLog.topics, data: depositedLog.data
        });

        bytes memory receiptChunk = abi.encode(uint8(1), uint64(21000), decodedLogs, bytes(""));
        bytes[] memory chunks = new bytes[](3);
        chunks[0] = "";
        chunks[1] = "";
        chunks[2] = receiptChunk;
        encodedTx = abi.encode(uint8(0), chunks);
    }

    function _emptyProofs()
        internal
        pure
        returns (
            BlockProverTypes.InclusionProof memory inclusion,
            BlockProverTypes.ContinuityProof memory continuity
        )
    {
        inclusion = BlockProverTypes.InclusionProof({
            kind: BlockProverTypes.ProofKind.BinaryMerkle, root: bytes32(0), data: ""
        });
        continuity = BlockProverTypes.ContinuityProof({
            lowerEndpointDigest: bytes32(0), roots: new bytes32[](0)
        });
    }

    function test_claim_publishesReleaseAndMarksClaimed() public {
        bytes memory encodedTx = _depositAndBuildEncodedTx(1 ether);
        verifier.setNextEncodedTransaction(encodedTx);
        (
            BlockProverTypes.InclusionProof memory inclusion,
            BlockProverTypes.ContinuityProof memory continuity
        ) = _emptyProofs();

        hub.claim(SRC_CHAIN_KEY, 100, inclusion, continuity);

        bytes32 key = keccak256(abi.encode(SRC_CHAIN_KEY, address(sourceVault), uint256(0)));
        assertTrue(hub.claimed(key));

        bytes memory expectedCalldata = abi.encodeWithSelector(
            IBridgeVaultRelease.releaseFromBridge.selector, key, recipient, address(0), 1 ether
        );
        bytes memory expectedEnvelope =
            EVMPayloadCodec.encode(destVaultPlaceholder, 0, RELEASE_GAS_LIMIT, expectedCalldata);
        assertEq(destOutbox.payloadLast(), expectedEnvelope);
        assertFalse(destOutbox.canAckLast());
        assertEq(destOutbox.callerLast(), address(hub));
    }

    function test_claim_revertsOnUnconfiguredSourceChain() public {
        bytes memory encodedTx = _depositAndBuildEncodedTx(1 ether);
        verifier.setNextEncodedTransaction(encodedTx);
        (
            BlockProverTypes.InclusionProof memory inclusion,
            BlockProverTypes.ContinuityProof memory continuity
        ) = _emptyProofs();

        vm.expectRevert(abi.encodeWithSelector(BridgeHub.ChainNotConfigured.selector, uint32(99)));
        hub.claim(99, 100, inclusion, continuity);
    }

    function test_claim_revertsOnWrongEmitter() public {
        // A vault deployed under a *different* address than the one BridgeHub has on file for
        // SRC_CHAIN_KEY — its Deposited log must not be accepted as that chain's deposit.
        BridgeVault impostor = new BridgeVault(inboxStub, owner, bridgeHubStub);
        vm.deal(depositor, 1 ether);
        vm.recordLogs();
        vm.prank(depositor);
        impostor.deposit{value: 1 ether}(address(0), 1 ether, DEST_CHAIN_KEY, recipient);
        Vm.Log[] memory logs = vm.getRecordedLogs();

        EvmV1Decoder.LogEntryTuple[] memory decodedLogs = new EvmV1Decoder.LogEntryTuple[](1);
        decodedLogs[0] = EvmV1Decoder.LogEntryTuple({
            address_: logs[0].emitter, topics: logs[0].topics, data: logs[0].data
        });
        bytes[] memory chunks = new bytes[](3);
        chunks[0] = "";
        chunks[1] = "";
        chunks[2] = abi.encode(uint8(1), uint64(21000), decodedLogs, bytes(""));
        verifier.setNextEncodedTransaction(abi.encode(uint8(0), chunks));

        (
            BlockProverTypes.InclusionProof memory inclusion,
            BlockProverTypes.ContinuityProof memory continuity
        ) = _emptyProofs();

        vm.expectRevert(
            abi.encodeWithSelector(
                BridgeHub.WrongEmitter.selector, address(impostor), address(sourceVault)
            )
        );
        hub.claim(SRC_CHAIN_KEY, 100, inclusion, continuity);
    }

    function test_claim_revertsOnDoubleClaim() public {
        bytes memory encodedTx = _depositAndBuildEncodedTx(1 ether);
        verifier.setNextEncodedTransaction(encodedTx);
        (
            BlockProverTypes.InclusionProof memory inclusion,
            BlockProverTypes.ContinuityProof memory continuity
        ) = _emptyProofs();

        hub.claim(SRC_CHAIN_KEY, 100, inclusion, continuity);

        vm.expectRevert(BridgeHub.AllDepositsAlreadyClaimed.selector);
        hub.claim(SRC_CHAIN_KEY, 100, inclusion, continuity);
    }

    function test_claim_revertsOnUnconfiguredDestinationChain() public {
        // Deposit targets a destChainKey BridgeHub has never configured.
        vm.deal(depositor, 1 ether);
        vm.recordLogs();
        vm.prank(depositor);
        sourceVault.deposit{value: 1 ether}(address(0), 1 ether, 77, recipient);
        Vm.Log[] memory logs = vm.getRecordedLogs();

        EvmV1Decoder.LogEntryTuple[] memory decodedLogs = new EvmV1Decoder.LogEntryTuple[](1);
        for (uint256 i; i < logs.length; ++i) {
            if (
                logs[i].emitter == address(sourceVault)
                    && logs[i].topics[0] == BridgeVault.Deposited.selector
            ) {
                decodedLogs[0] = EvmV1Decoder.LogEntryTuple({
                    address_: logs[i].emitter, topics: logs[i].topics, data: logs[i].data
                });
            }
        }
        bytes[] memory chunks = new bytes[](3);
        chunks[0] = "";
        chunks[1] = "";
        chunks[2] = abi.encode(uint8(1), uint64(21000), decodedLogs, bytes(""));
        verifier.setNextEncodedTransaction(abi.encode(uint8(0), chunks));

        (
            BlockProverTypes.InclusionProof memory inclusion,
            BlockProverTypes.ContinuityProof memory continuity
        ) = _emptyProofs();

        vm.expectRevert(abi.encodeWithSelector(BridgeHub.ChainNotConfigured.selector, uint32(77)));
        hub.claim(SRC_CHAIN_KEY, 100, inclusion, continuity);
    }

    function test_setChainConfig_onlyOwner() public {
        vm.expectRevert(
            abi.encodeWithSignature("OwnableUnauthorizedAccount(address)", address(this))
        );
        hub.setChainConfig(SRC_CHAIN_KEY, address(sourceVault), address(destOutbox), true);
    }
}
