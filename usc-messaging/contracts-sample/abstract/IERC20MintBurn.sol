// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

interface IERC20Burnable {
    function burn(uint256 value) external;
}

interface IERC20Mintable {
    function mint(address to, uint256 value) external;
}
