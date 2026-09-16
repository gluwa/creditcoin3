// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {Ownable2Step, Ownable} from "@openzeppelin/contracts/access/Ownable2Step.sol";
import {Pausable} from "@openzeppelin/contracts/utils/Pausable.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

import {IASCProofVerifier} from "@gluwa/usc-contracts/write-ability/abstract/IASCProofVerifier.sol";
import {IOutbox} from "@gluwa/usc-contracts/write-ability/abstract/IOutbox.sol";
import {BlockProverTypes} from "@gluwa/usc-contracts/write-ability/common/BlockProverTypes.sol";
import {EvmV1Decoder} from "@gluwa/usc-contracts/common/EvmV1Decoder.sol";
import {EVMPayloadCodec} from "@gluwa/usc-contracts/write-ability/common/EVMPayloadCodec.sol";

/// @dev Just enough of BridgeVault's ABI to compute `releaseFromBridge`'s selector — importing the
///      whole contract isn't needed, and would pull its own inheritance chain into this file.
interface IBridgeVaultRelease {
    function releaseFromBridge(bytes32 messageId, address recipient, address token, uint256 amount)
        external;
}

/// @notice Creditcoin-side hub, deployed once. Verifies a native USC proof of a `Deposited` event
///         emitted by a spoke chain's BridgeVault, then republishes it as an unpaid, unacked
///         Outbox message toward the destination spoke's BridgeVault. `claim` is permissionless —
///         a standalone off-chain claimer bot normally calls it, but anyone with gas may
///         (`claimed` makes double-submission a safe no-op).
/// @dev Deliberately skips the paid-relay tier entirely (RelayerContractLite / quoter EOA /
///      AcknowledgmentValidator): BridgeVault.Released is externally observable, so BridgeHub
///      never needs Outbox-level delivery confirmation, and always publishes with `canAck = false`.
///      Publishes through each destination's existing, shared Outbox/Inbox/DispatcherRouter — see
///      BridgeVault.sol's NatSpec for why no dedicated Inbox/Outbox of our own is needed.
contract BridgeHub is Ownable2Step, Pausable {
    /// @dev DispatcherRouter/DispatcherBase revert on a zero gas limit and forward exactly this
    ///      much gas to BridgeVault.releaseFromBridge's call (asc-contracts write-ability/common/
    ///      DestinationCall.sol) — sized with margin over the profiled native/ERC20 release path.
    uint256 private constant RELEASE_GAS_LIMIT = 200_000;

    struct ChainConfig {
        address vault;
        address outbox;
        bool enabled;
    }

    /// @dev Must match BridgeVault's `Deposited(uint256 indexed nonce, address indexed depositor,
    ///      address indexed recipient, uint32 destChainKey, address token, uint256 amount)`
    ///      exactly — this is a topic0, not a Solidity type, so drift is silent (matches nothing)
    ///      rather than a compile error.
    bytes32 private constant DEPOSITED_SIG =
        keccak256("Deposited(uint256,address,address,uint32,address,uint256)");

    IASCProofVerifier public proofVerifier;
    IERC20 public immutable attestToken;

    /// @notice Per-registry-chain-key config (the same numeric chain key used by
    ///         Outbox.chainKey()/OutboxFactory, distinct from the Inbox's write-ability
    ///         `bytes32` chain key used only for internal vote validation).
    mapping(uint32 => ChainConfig) public chainConfigs;

    /// @notice keccak256(abi.encode(sourceChainKey, vault, nonce)) => already claimed. Chain-key
    ///         scoped so two spoke chains can never collide on the same nonce.
    mapping(bytes32 => bool) public claimed;

    event ChainConfigSet(uint32 indexed chainKey, address vault, address outbox, bool enabled);
    event ProofVerifierSet(address indexed oldVerifier, address indexed newVerifier);
    event Claimed(
        bytes32 indexed key,
        uint32 indexed sourceChainKey,
        uint32 indexed destChainKey,
        address recipient,
        address token,
        uint256 amount,
        bytes32 messageId
    );

    error ZeroAddress();
    error ChainNotConfigured(uint32 chainKey);
    error TransactionFailed();
    error NoDepositedLogs();
    error WrongEmitter(address got, address expected);
    error AllDepositsAlreadyClaimed();

    constructor(address proofVerifier_, address attestToken_, address initialOwner)
        Ownable(initialOwner)
    {
        if (proofVerifier_ == address(0) || attestToken_ == address(0)) revert ZeroAddress();
        proofVerifier = IASCProofVerifier(proofVerifier_);
        attestToken = IERC20(attestToken_);
    }

    /// @notice Verify a proven `Deposited` transaction from `sourceChainKey` and republish each
    ///         not-yet-claimed deposit it contains as a release message toward its destination
    ///         chain's Outbox. Reverts if every deposit in the proof was already claimed, so a
    ///         racing bot/user submission is a cheap no-op rather than a silent success.
    function claim(
        uint32 sourceChainKey,
        uint64 blockHeight,
        BlockProverTypes.InclusionProof calldata inclusionProof,
        BlockProverTypes.ContinuityProof calldata continuityProof
    ) external whenNotPaused {
        ChainConfig memory src = chainConfigs[sourceChainKey];
        if (!src.enabled) revert ChainNotConfigured(sourceChainKey);

        bytes memory encodedTx = proofVerifier.verifyProofs(
            bytes32(uint256(sourceChainKey)), blockHeight, inclusionProof, continuityProof
        );

        EvmV1Decoder.ReceiptFields memory receipt = EvmV1Decoder.decodeReceiptFields(encodedTx);
        if (receipt.receiptStatus != 1) revert TransactionFailed();

        EvmV1Decoder.LogEntry[] memory logs =
            EvmV1Decoder.getLogsByEventSignature(receipt, DEPOSITED_SIG);
        if (logs.length == 0) revert NoDepositedLogs();

        uint256 processed;
        for (uint256 i; i < logs.length; ++i) {
            if (_claimOne(sourceChainKey, src.vault, logs[i])) {
                unchecked {
                    ++processed;
                }
            }
        }

        if (processed == 0) revert AllDepositsAlreadyClaimed();
    }

    /// @dev Split out of `claim` to keep that function's stack shallow enough to compile without
    ///      `via_ir`. Returns false (a safe no-op) for an already-claimed deposit.
    function _claimOne(uint32 sourceChainKey, address vault, EvmV1Decoder.LogEntry memory log)
        private
        returns (bool)
    {
        if (log.address_ != vault) revert WrongEmitter(log.address_, vault);

        uint256 nonce = uint256(log.topics[1]);
        address recipient = address(uint160(uint256(log.topics[3])));
        (uint32 destChainKey, address token, uint256 amount) =
            abi.decode(log.data, (uint32, address, uint256));

        bytes32 key = keccak256(abi.encode(sourceChainKey, vault, nonce));
        if (claimed[key]) return false;
        claimed[key] = true;

        ChainConfig memory dst = chainConfigs[destChainKey];
        if (!dst.enabled) revert ChainNotConfigured(destChainKey);

        bytes memory releaseCalldata = abi.encodeWithSelector(
            IBridgeVaultRelease.releaseFromBridge.selector, key, recipient, token, amount
        );
        bytes memory envelope =
            EVMPayloadCodec.encode(dst.vault, 0, RELEASE_GAS_LIMIT, releaseCalldata);
        bytes32 messageId = IOutbox(dst.outbox).publishMessage(false, envelope);

        emit Claimed(key, sourceChainKey, destChainKey, recipient, token, amount, messageId);
        return true;
    }

    function setChainConfig(uint32 chainKey, address vault, address outbox, bool enabled)
        external
        onlyOwner
    {
        if (vault == address(0) || outbox == address(0)) revert ZeroAddress();
        chainConfigs[chainKey] = ChainConfig({vault: vault, outbox: outbox, enabled: enabled});
        emit ChainConfigSet(chainKey, vault, outbox, enabled);
    }

    function setProofVerifier(address newVerifier) external onlyOwner {
        if (newVerifier == address(0)) revert ZeroAddress();
        emit ProofVerifierSet(address(proofVerifier), newVerifier);
        proofVerifier = IASCProofVerifier(newVerifier);
    }

    /// @notice Sets (or refreshes) this hub's ATTEST allowance to `outbox`, needed only if that
    ///         chain's coreFee is ever set non-zero — `publishMessage` skips the pull entirely
    ///         when the registry's coreFee is 0, which is the expected default for this POC.
    function approveOutboxFee(address outbox, uint256 amount) external onlyOwner {
        attestToken.approve(outbox, amount);
    }

    function pause() external onlyOwner {
        _pause();
    }

    function unpause() external onlyOwner {
        _unpause();
    }
}
