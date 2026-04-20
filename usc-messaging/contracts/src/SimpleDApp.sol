// This contract plays the role of a dApp contract deployed on Creditcoin
// by a builder team. DApp contracts could trigger message passing via
// writability for many reasons, but in our example we model the simplest
// possible reason. An end user makes a dApp contract call requesting to 
// bridge funds from one chain to another.

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "@openzeppelin/contracts/token/ERC20/ERC20.sol";

interface IOutbox {
    function publishMessage(
        bool requiresAck,
        bytes calldata payload
    ) external returns (bytes32 messageId);
}

/// @notice Example ERC20 dApp using one big pooled escrow.
/// @dev Dummy example only. Assumes inbound delivery authenticity and replay
///      protection are handled elsewhere.
contract SimpleDApp is ERC20 {
    address public owner;
    address public outboxAddr;
    IOutbox public outbox;

    /// @notice Total tokens currently held in the escrow pool.
    uint256 public totalEscrowPool;

    /// @notice Just for observability / debugging.
    mapping(bytes32 => bool) public messageDispatched;
    mapping(bytes32 => bool) public messageDelivered;

    event MessageDispatched(bytes32 indexed messageId);
    event MessageDelivered(bytes32 indexed messageId);

    event TokensEscrowed(
        bytes32 indexed messageId,
        uint256 amount
    );

    event TokensRedeemed(
        address indexed recipient,
        uint256 amount
    );

    modifier onlyOwner() {
        require(msg.sender == owner, "Not owner");
        _;
    }

    constructor(address _outboxAddr) ERC20("SimpleDApp Token", "SDT") {
        require(_outboxAddr != address(0), "Invalid outbox address");

        owner = msg.sender;
        outboxAddr = _outboxAddr;
        outbox = IOutbox(_outboxAddr);

        _mint(msg.sender, 10_000 * 10**decimals());
    }

    /// @dev Use 18 decimals so amounts naturally represent "micro units".
    function decimals() public pure override returns (uint8) {
        return 18;
    }

    /// @notice Locks tokens into the shared escrow pool and publishes a cross-chain message.
    /// @param requiresAck Whether the outbox message requires acknowledgement.
    /// @param destinationContract Destination contract address to encode into the payload.
    /// @param recipient Address on the destination chain to be credited with tokens
    /// @param amount Token amount in micro units.
    function sendTokens(
        bool requiresAck,
        address destinationContract,
        address recipient,
        uint256 amount
    ) external returns (bytes32 messageId) {
        require(destinationContract != address(0), "Invalid destination");
        require(recipient != address(0), "Invalid recipient");
        require(amount > 0, "Amount must be > 0");

        // Move tokens from sender into the single pooled escrow.
        _transfer(msg.sender, address(this), amount);
        totalEscrowPool += amount;

        // Encode recipient + amount for the destination chain.
        bytes memory payloadData = abi.encode(recipient, amount);
        bytes memory payload = abi.encode(destinationContract, payloadData);

        messageId = outbox.publishMessage(requiresAck, payload);

        messageDispatched[messageId] = true;

        emit TokensEscrowed(messageId, amount);
        emit MessageDispatched(messageId);
    }

    /// @notice Marks a dispatched message as delivered.
    function markDelivered(bytes32 messageId) external onlyOwner {
        require(messageDispatched[messageId], "Unknown messageId");
        messageDelivered[messageId] = true;

        emit MessageDelivered(messageId);
    }

    /// @notice Redeems caller's claimable tokens from the pooled escrow.
    function redeemTokens(uint256 amount, address recipient) external onlyOwner {
        require(amount > 0, "Amount must be > 0");
        require(recipient != address(0), "Invalid recipient");
        require(totalEscrowPool >= amount, "Insufficient pool");

        totalEscrowPool -= amount;

        _transfer(address(this), recipient, amount);

        emit TokensRedeemed(recipient, amount);
    }
}