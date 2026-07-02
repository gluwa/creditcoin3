// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

interface IERC20 {
    function transfer(address to, uint256 amount) external returns (bool);
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
}

/// @notice Anvil-side endpoint of the 2-way USC token bridge.
///
/// **Outbound (Anvil → Creditcoin), proof-based:** `lock` escrows tokens and emits `Locked`. A user
/// then proves that transaction to `CcBridge.claim` on Creditcoin via the block-prover precompile —
/// no attestor votes, because Creditcoin natively attests Anvil blocks.
///
/// **Inbound (Creditcoin → Anvil), vote-based:** the USC `Inbox` delivers a release message to
/// `receiveMessage` (after attestor quorum), which pays out escrowed liquidity. The Inbox is the only
/// authorized caller.
contract AnvilBridge {
    IERC20 public immutable token;
    /// USC Inbox authorized to deliver release messages here (CC → Anvil direction).
    address public immutable inbox;

    /// Monotonic per-lock nonce, bound into `Locked` so each lock proof is unique/deduped on CC.
    uint256 public lockNonce;
    /// CC → Anvil delivery dedup, keyed by USC `messageId`.
    mapping(bytes32 => bool) public processedMessages;

    event Locked(address indexed ccRecipient, uint256 amount, uint256 nonce);
    event Released(address indexed recipient, uint256 amount, bytes32 indexed messageId);

    error NotInbox();
    error AlreadyProcessed();

    constructor(address _token, address _inbox) {
        require(_token != address(0) && _inbox != address(0), "zero arg");
        token = IERC20(_token);
        inbox = _inbox;
    }

    /// Escrow `amount` from the caller and signal that `ccRecipient` should receive bridged tokens on
    /// Creditcoin. The caller must have approved this contract for `amount` first.
    function lock(uint256 amount, address ccRecipient) external {
        require(ccRecipient != address(0), "zero recipient");
        require(token.transferFrom(msg.sender, address(this), amount), "transferFrom");
        emit Locked(ccRecipient, amount, lockNonce++);
    }

    /// USC delivery callback. The signature (selector) must match `SimpleInbox.executeDelivery`'s
    /// `receiveMessage(bytes32,uint256,address,bytes)`. Releases escrowed liquidity to the recipient
    /// encoded in `payload`.
    function receiveMessage(
        bytes32 messageId,
        uint256, /* srcChainId */
        address, /* emitter */
        bytes calldata payload
    )
        external
    {
        if (msg.sender != inbox) revert NotInbox();
        if (processedMessages[messageId]) revert AlreadyProcessed();
        processedMessages[messageId] = true;

        (address recipient, uint256 amount) = abi.decode(payload, (address, uint256));
        require(token.transfer(recipient, amount), "transfer");
        emit Released(recipient, amount, messageId);
    }
}
