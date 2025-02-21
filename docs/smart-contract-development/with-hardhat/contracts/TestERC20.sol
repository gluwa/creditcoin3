// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "@openzeppelin/contracts/token/ERC20/ERC20.sol";

contract TestERC20 is ERC20 {
    constructor() ERC20("Burn Test", "TEST") {
        _mint(msg.sender, 1000000000000000000000);
    }
}
