// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.20;

import {Test, console} from "forge-std/Test.sol";
import {TestERC20} from "../src/TestERC20.sol";

contract TestERCTest is Test {
    TestERC20 public testERC20;

    function setUp() public {
        testERC20 = new TestERC20();
    }

    function testTransfer() public {
        // Transfer to burn address
        testERC20.transfer(0x0000000000000000000000000000000000000001, 100000);
    }
}
