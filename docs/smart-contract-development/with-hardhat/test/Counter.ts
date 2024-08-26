import { expect } from 'chai';
import { ethers } from 'hardhat';
import { Contract } from 'ethers';

describe('Counter contract', function () {
    let counter: Contract;

    beforeEach(async function () {
        // Deploy the Counter contract before the tests
        const CounterFactory = await ethers.getContractFactory("Counter");
        counter = await CounterFactory.deploy();
        await counter.waitForDeployment();
    });

    it("Should be deployed with the initial value 0", async function ()
    {
        const count = await counter.getCount();
        expect(count).to.equal(0);
    });

    it('Should increment count by 1', async function () {
        await counter.incrementCounter();
        expect(await counter.getCount()).to.equal(1);
    });
});
