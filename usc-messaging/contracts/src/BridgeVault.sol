// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {TrustedInboxBase} from "@gluwa/usc-contracts/write-ability/abstract/TrustedInboxBase.sol";
import {CompatibleERC20} from "@gluwa/usc-contracts/write-ability/common/CompatibleERC20.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

/// @notice Lock-and-release vault deployed on each bridge spoke chain (Base Sepolia, Ethereum
///         Sepolia). Plays both roles simultaneously: the lock point when this chain is a
///         deposit's source, and the release point when this chain is a deposit's destination.
/// @dev Release delivery rides on asc-contracts' shared `DispatcherRouter`/`DefaultDispatcher`
///      infra already deployed and already wired to this chain's live Inbox and the running
///      message-relayer — no dedicated Inbox/Outbox/EOAValidator of our own. `DestinationCall`
///      (the router's execution primitive) makes a plain external call here with the
///      attestation-authenticated emitter appended as the trailing 20 bytes of calldata (an
///      ERC-2771-style trusted-forwarder pattern), so `releaseFromBridge` authenticates by
///      combining `msg.sender == the trusted router` (this chain's `DispatcherRouter`, trusted via
///      `TrustedInboxBase`, set at construction) with the extracted trailing emitter `==
///      bridgeHub` — replacing the old design's single "trusted Inbox + IMessageReceiver" story.
contract BridgeVault is TrustedInboxBase {
    using CompatibleERC20 for IERC20;

    /// @notice The BridgeHub contract on Creditcoin trusted to emit release messages.
    address public bridgeHub;

    /// @notice ERC-20 tokens this vault will lock/release. address(0) (native ETH) is always
    ///         allowed and is not tracked here.
    mapping(address => bool) public allowedTokens;

    /// @notice Monotonic per-vault deposit counter. Combined with (sourceChainKey, this vault's
    ///         address) on BridgeHub, it alone is sufficient replay-key material — no need to
    ///         also hash recipient/amount into the claim key.
    uint256 public depositNonce;

    /// @notice Replay guard for `releaseFromBridge`, keyed by the same `messageId` BridgeHub uses
    ///         as its own claim key — DispatcherRouter's own message-state machine already
    ///         guarantees at-most-once delivery per messageId, but this is cheap, self-contained
    ///         defense in depth rather than relying solely on upstream infra we don't own.
    mapping(bytes32 => bool) public processedMessages;

    /// @dev Field order/indexing is load-bearing: BridgeHub.claim decodes these logs directly
    ///      out of a proven receipt (see BridgeHub's DEPOSITED_SIG), and the dApp's history
    ///      screen filters getLogs by `depositor` or `recipient` topics — do not reorder without
    ///      updating both.
    event Deposited(
        uint256 indexed nonce,
        address indexed depositor,
        address indexed recipient,
        uint32 destChainKey,
        address token,
        uint256 amount
    );
    event Released(
        bytes32 indexed messageId, address indexed recipient, address token, uint256 amount
    );
    event BridgeHubSet(address indexed oldHub, address indexed newHub);
    event AllowedTokenSet(address indexed token, bool allowed);

    error ZeroAmount();
    error ZeroRecipient();
    error ZeroBridgeHub();
    error NativeAmountMismatch(uint256 expected, uint256 got);
    error UnsupportedToken(address token);
    error UntrustedEmitter(address got, address expected);
    error InsufficientLiquidity(address token, uint256 requested, uint256 available);
    error NativeReleaseFailed(address recipient, uint256 amount);
    error MessageAlreadyProcessed(bytes32 messageId);

    constructor(address initialInbox, address initialOwner, address bridgeHub_)
        TrustedInboxBase(initialInbox, initialOwner)
    {
        if (bridgeHub_ == address(0)) revert ZeroBridgeHub();
        bridgeHub = bridgeHub_;
    }

    /// @notice Accepts plain-transfer liquidity top-ups (operator funding native ETH). No special
    ///         "fund" entry point is needed for the lock-and-release model.
    receive() external payable {}

    /// @notice Locks `amount` of `token` (address(0) = native ETH) for release to `recipient` on
    ///         the spoke chain identified by `destChainKey`.
    function deposit(address token, uint256 amount, uint32 destChainKey, address recipient)
        external
        payable
    {
        if (amount == 0) revert ZeroAmount();
        if (recipient == address(0)) revert ZeroRecipient();

        if (token == address(0)) {
            if (msg.value != amount) revert NativeAmountMismatch(amount, msg.value);
        } else {
            if (msg.value != 0) revert NativeAmountMismatch(0, msg.value);
            if (!allowedTokens[token]) revert UnsupportedToken(token);
            IERC20(token).compatibleTransferFrom(msg.sender, address(this), amount);
        }

        uint256 nonce = depositNonce++;
        emit Deposited(nonce, msg.sender, recipient, destChainKey, token, amount);
    }

    /// @notice Release entry point called by this chain's DispatcherRouter (via DefaultDispatcher)
    ///         once BridgeHub's release message has been attested and delivered through the
    ///         existing, shared Inbox.
    /// @dev IMPORTANT — no automatic retry on revert here, unlike the old dedicated-Inbox design.
    ///      DestinationCall.tryCall (asc-contracts write-ability/common/DestinationCall.sol) calls
    ///      this with nativeCoinValue == 0 (we release the vault's own held liquidity, not value
    ///      carried by the call), and for a zero-value call ANY revert here — including
    ///      InsufficientLiquidity — is swallowed into DispatcherTypes.MessageState.Failed, which
    ///      Inbox.deliverMessage marks as a *terminal, completed* delivery (DeliveryStatus.
    ///      DestinationFailed), not a pending one — retryPendingMessage is never reachable for it.
    ///      Verified empirically against the real DispatcherRouter/DefaultDispatcher/Inbox
    ///      contracts, not just read from source. Accepted as a known POC limitation: keep
    ///      destination vaults well-funded ahead of claims rather than relying on retry.
    function releaseFromBridge(bytes32 messageId, address recipient, address token, uint256 amount)
        external
    {
        _requireTrustedInbox(msg.sender);
        if (processedMessages[messageId]) revert MessageAlreadyProcessed(messageId);
        processedMessages[messageId] = true;

        address emitterAddress = _trailingEmitter();
        if (emitterAddress != bridgeHub) revert UntrustedEmitter(emitterAddress, bridgeHub);

        _release(recipient, token, amount);
        emit Released(messageId, recipient, token, amount);
    }

    /// @dev `DestinationCall.tryCall` (asc-contracts write-ability/common/DestinationCall.sol)
    ///      appends the attestation-authenticated emitter as the trailing 20 bytes of calldata —
    ///      the same ERC-2771 trusted-forwarder pattern as OpenZeppelin's `ERC2771Context.
    ///      _msgSender()`. Solidity's own argument decoding for `releaseFromBridge`'s four
    ///      fixed-size params only consumes the leading `4 + 32*4` bytes, so reading from the end
    ///      of calldata isolates exactly the appended bytes regardless of layout.
    function _trailingEmitter() private pure returns (address emitterAddress) {
        assembly {
            emitterAddress := shr(96, calldataload(sub(calldatasize(), 20)))
        }
    }

    function _release(address recipient, address token, uint256 amount) private {
        if (token == address(0)) {
            if (address(this).balance < amount) {
                revert InsufficientLiquidity(token, amount, address(this).balance);
            }
            (bool ok,) = payable(recipient).call{value: amount}("");
            if (!ok) revert NativeReleaseFailed(recipient, amount);
        } else {
            uint256 balance = IERC20(token).balanceOf(address(this));
            if (balance < amount) revert InsufficientLiquidity(token, amount, balance);
            IERC20(token).compatibleTransfer(recipient, amount);
        }
    }

    function setBridgeHub(address newHub) external onlyOwner {
        if (newHub == address(0)) revert ZeroBridgeHub();
        emit BridgeHubSet(bridgeHub, newHub);
        bridgeHub = newHub;
    }

    function setAllowedToken(address token, bool allowed) external onlyOwner {
        allowedTokens[token] = allowed;
        emit AllowedTokenSet(token, allowed);
    }
}
