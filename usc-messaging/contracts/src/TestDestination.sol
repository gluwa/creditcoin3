// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "@openzeppelin/contracts/token/ERC20/ERC20.sol";

/// @notice Test destination ERC20 for PoC. Receives delivered messages and mints tokens.
contract TestDestination is ERC20 {
    address public owner;
    address public inbox;

    event TokensBridged(
        bytes32 indexed messageId,
        address indexed emitterAddress,
        address indexed recipient,
        uint256 amount
    );

    event TokensBurnedForBridging(
        address indexed from,
        uint256 amount
    );

    modifier onlyInbox() {
        require(msg.sender == inbox, "Not inbox");
        _;
    }

    constructor(address _inbox) ERC20("Test Destination Token", "TDT") {
        require(_inbox != address(0), "Invalid inbox");
        owner = msg.sender;
        inbox = _inbox;
    }

    function decimals() public pure override returns (uint8) {
        return 18;
    }

    function receiveMessage(
        bytes32 messageId,
        uint256 creditcoinChainId,
        address emitterAddress,
        bytes calldata payload
    ) external onlyInbox {
        creditcoinChainId; // silence unused parameter warning

        // Inbox already decoded the outer wrapper:
        // abi.encode(destinationContract, payloadData)
        // So payload here is just:
        // abi.encode(recipient, amount)
        (address recipient, uint256 amount) = abi.decode(payload, (address, uint256));

        require(recipient != address(0), "Invalid recipient");
        require(amount > 0, "Amount must be > 0");

        _mint(recipient, amount);

        emit TokensBridged(messageId, emitterAddress, recipient, amount);
    }

    /// @notice Burns tokens from the caller so they can be bridged out.
    /// @param amount Amount of token base units to burn.
    function sendTokens(uint256 amount) external {
        require(amount > 0, "Amount must be > 0");

        _burn(msg.sender, amount);

        emit TokensBurnedForBridging(msg.sender, amount);
    }
}
