// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {
    Ownable2Step,
    Ownable
} from "@openzeppelin/contracts/access/Ownable2Step.sol";
import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {ERC20Burnable} from "@openzeppelin/contracts/token/ERC20/extensions/ERC20Burnable.sol";

contract MockBridgeToken is ERC20Burnable, Ownable2Step {
    mapping(address => bool) public minters;

    constructor(address initialOwner) ERC20("Mock Bridge Token", "MBT") Ownable(initialOwner) {}

    function mint(address to, uint256 amount) external {
        require(
            msg.sender == owner() || minters[msg.sender],
            "MockBridgeToken: unauthorized minter"
        );
        _mint(to, amount);
    }

    function setMinter(address minter, bool allowed) external onlyOwner {
        minters[minter] = allowed;
    }

}
