// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "./MockAttestToken.sol";

/// ERC-20 test double that returns true but credits one wei less than requested on transferFrom.
contract FeeOnTransferAttestToken is MockAttestToken {
    function transferFrom(address from, address to, uint256 amount) public override returns (bool) {
        uint256 a = allowance[from][msg.sender];
        require(a >= amount, "insufficient allowance");
        allowance[from][msg.sender] = a - amount;
        require(balanceOf[from] >= amount, "insufficient balance");
        balanceOf[from] -= amount;
        balanceOf[to] += amount - 1;
        emit Transfer(from, to, amount - 1);
        return true;
    }
}
