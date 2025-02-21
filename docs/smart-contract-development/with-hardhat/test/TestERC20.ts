import { ethers } from 'hardhat';
const { expect } = require("chai");

describe("Test erc 20", function () {
  // ...previous test...

  it("Token burn should work", async function() {
    const [owner, addr1, addr2] = await ethers.getSigners();

    const TestERC20 = await ethers.deployContract("TestERC20");

    // Transfer 50 tokens from owner to addr1
    await TestERC20.transfer("0x0000000000000000000000000000000000000001", 100000);
    expect(await TestERC20.balanceOf("0x0000000000000000000000000000000000000001")).to.equal(100000);
  });
});
