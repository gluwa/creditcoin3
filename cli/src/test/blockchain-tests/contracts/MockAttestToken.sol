// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// Minimal mintable token for attest-coin rewards tests (matches `mint(address,uint256)` entry used by the precompile).
contract MockAttestToken {
    mapping(address => uint256) public balanceOf;
    address public minter;

    event Transfer(address indexed from, address indexed to, uint256 value);

    constructor() {
        minter = msg.sender;
    }

    function setMinter(address newMinter) external {
        require(msg.sender == minter, "not minter");
        minter = newMinter;
    }

    function mint(address to, uint256 amount) external {
        require(msg.sender == minter, "not minter");
        balanceOf[to] += amount;
        emit Transfer(address(0), to, amount);
    }
}
