// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "node_modules/@openzeppelin/contracts/token/ERC20/ERC20.sol";

contract TestERC20 is ERC20 {
    constructor() ERC20("Burn Test", "TEST") {
        _mint(msg.sender, 452978427939818775018056643980856);
    }
}
