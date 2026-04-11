// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {
    Ownable2Step,
    Ownable
} from "@openzeppelin/contracts/access/Ownable2Step.sol";

import {IERC20Mintable} from "./abstract/IERC20MintBurn.sol";

/// @title USC Bridge Mint Destination
/// @notice Network-usable destination contract called by `USCBridgeLiquidityOperator.bridgeFromIntent`.
/// @dev This contract must be granted the required mint role on the configured USC token/bridge contract.
contract USCBridgeMintDestination is Ownable2Step {
    error ZeroAddress();
    error UnauthorizedCaller(address caller);
    error UnsupportedMintToken(address token);
    error InvalidRecipient();
    error InvalidAmount();

    /// @notice Emitted when the USC bridge operator is updated.
    event BridgeOperatorSet(address indexed operator);

    /// @notice Emitted when a mintable token is allowed/blocked.
    event MintTokenSet(address indexed token, bool allowed);

    /// @notice Emitted after successful mint execution.
    event MintExecuted(
        bytes32 indexed intentId,
        address indexed token,
        address indexed recipient,
        uint256 amount,
        address caller
    );

    /// @notice USC bridge operator allowed to invoke mint execution.
    address public bridgeOperator;

    /// @notice Owner-managed allowlist of token/bridge contracts exposing `mint(address,uint256)`.
    mapping(address => bool) public mintTokenAllowed;

    /// @param initialBridgeOperator Initial authorized USC bridge operator.
    /// @param initialMintToken Initial mint token/bridge contract allowed for mint execution.
    /// @param initialOwner Initial contract owner.
    constructor(
        address initialBridgeOperator,
        address initialMintToken,
        address initialOwner
    ) Ownable(initialOwner) {
        if (
            initialBridgeOperator == address(0) ||
            initialMintToken == address(0) ||
            initialOwner == address(0)
        ) {
            revert ZeroAddress();
        }

        bridgeOperator = initialBridgeOperator;
        mintTokenAllowed[initialMintToken] = true;

        emit BridgeOperatorSet(initialBridgeOperator);
        emit MintTokenSet(initialMintToken, true);
    }

    /// @notice Updates the authorized USC bridge operator caller.
    /// @param operator New operator address.
    function setBridgeOperator(address operator) external onlyOwner {
        if (operator == address(0)) {
            revert ZeroAddress();
        }
        bridgeOperator = operator;
        emit BridgeOperatorSet(operator);
    }

    /// @notice Allows or blocks a token/bridge contract for mint execution.
    /// @param token Token/bridge address exposing `mint(address,uint256)`.
    /// @param allowed True to allow, false to block.
    function setMintToken(address token, bool allowed) external onlyOwner {
        if (token == address(0)) {
            revert ZeroAddress();
        }
        mintTokenAllowed[token] = allowed;
        emit MintTokenSet(token, allowed);
    }

    /// @notice Executes minting for a validated inbound intent.
    /// @dev This is intended to be called by `USCBridgeLiquidityOperator` via destinationCallData.
    /// @param intentId Inbound intent identifier for traceability.
    /// @param token Token/bridge contract to mint from (must be allowed by owner).
    /// @param recipient Recipient address to receive minted tokens.
    /// @param amount Amount to mint.
    function executeMint(
        bytes32 intentId,
        address token,
        address recipient,
        uint256 amount
    ) external {
        if (msg.sender != bridgeOperator) {
            revert UnauthorizedCaller(msg.sender);
        }
        if (!mintTokenAllowed[token]) {
            revert UnsupportedMintToken(token);
        }
        if (recipient == address(0)) {
            revert InvalidRecipient();
        }
        if (amount == 0) {
            revert InvalidAmount();
        }

        IERC20Mintable(token).mint(recipient, amount);
        emit MintExecuted(intentId, token, recipient, amount, msg.sender);
    }
}
