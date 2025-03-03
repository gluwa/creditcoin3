import { expect } from 'chai';
import { ethers } from 'hardhat';
import { Signer } from 'ethers';
import { ProverForTesting } from '../typechain-types';

describe('CreditcoinPublicProver', function () {
    let prover: ProverForTesting;
    let owner: Signer;
    let user: Signer;
    let proceedsAccount: Signer;
    let queryCost: bigint;
    const sampleQuery = {
        chainId: 1,
        height: 1000n,
        index: 0,
        layoutSegments: [
            { offset: 0, size: 32 },
            { offset: 32, size: 64 },
        ],
    };

    // WARNING: using a high-level beforeEach() instead of before() b/c we want contract
    // to be redeployed for every test scenario so that each scenario starts in a fresh state!
    // Otherwise we'll have to deal with tracking internal contract state & removing queries
    beforeEach(async function () {
        [owner, user, proceedsAccount] = await ethers.getSigners();

        // NOTE: interacting with a contract that inherits the SUT b/c it exposes
        // additional helper methods, like mock_setQueryState() for example
        const proverFactory = await ethers.getContractFactory('ProverForTesting');
        prover = await proverFactory.deploy(await proceedsAccount.getAddress(), 10n, 1000n, sampleQuery.chainId);
        await prover.waitForDeployment();

        queryCost = await prover.computeQueryCost(sampleQuery);
    });

    describe('Deployment', function () {
        it('Should set the right owner', async function () {
            expect(await prover.owner()).to.equal(await owner.getAddress());
        });

        it('Should initialize with zero total escrow balance', async function () {
            const totalEscrowBalance = await prover.getTotalEscrowBalance();
            expect(totalEscrowBalance).to.equal(0);
        });
    });

    describe('updateCostPerByte()', function () {
        it('Should store new cost and emit an event', async function () {
            const defaultCost = await prover.costPerByte();
            // configured in the contract constructor
            expect(defaultCost).to.equal(10n);

            // now let's change it
            const tx = await prover.connect(owner).updateCostPerByte(100n);
            const receipt = await tx.wait();
            const event = receipt?.logs[0];
            // @ts-ignore
            expect(event?.fragment.name).to.equal('CostPerByteUpdated');

            const newCost = await prover.costPerByte();
            expect(newCost).to.equal(100n);

            // subsequent query cost calculation uses the new cost/byte
            const newQueryCost = await prover.computeQueryCost(sampleQuery);
            const expectedCost = 1000n + 96n * 100n;
            expect(newQueryCost).to.equal(expectedCost);
        });

        it('Does not allow calls from non-owner', async function () {
            await expect(prover.connect(user).updateCostPerByte(100n)).to.be.revertedWith('Caller is not the owner');
        });
    });

    describe('updateBaseFee()', function () {
        it('Should store new fee and emit an event', async function () {
            const defaultFee = await prover.baseFee();
            // configured in the contract constructor
            expect(defaultFee).to.equal(1000n);

            // now let's change it
            const tx = await prover.connect(owner).updateBaseFee(100n);
            const receipt = await tx.wait();
            const event = receipt?.logs[0];
            // @ts-ignore
            expect(event?.fragment.name).to.equal('BaseFeeUpdated');

            const newFee = await prover.baseFee();
            expect(newFee).to.equal(100n);

            // subsequent query cost calculation uses the new fee
            const newQueryCost = await prover.computeQueryCost(sampleQuery);
            const expectedCost = 100n + 96n * 10n;
            expect(newQueryCost).to.equal(expectedCost);
        });

        it('Does not allow calls from non-owner', async function () {
            await expect(prover.connect(user).updateBaseFee(100n)).to.be.revertedWith('Caller is not the owner');
        });
    });

    describe('Query Cost Computation', function () {
        it('Should compute correct query cost based on layout segments', function () {
            const expectedCost = 1000n + 96n * 10n;
            expect(queryCost).to.equal(expectedCost);
        });
    });

    describe('submitQuery()', function () {
        it('Should accept query with sufficient payment', async function () {
            const escrowBeforeSubmit = await prover.getTotalEscrowBalance();
            expect(escrowBeforeSubmit).to.equal(0);

            const balanceBeforeSubmit = await ethers.provider.getBalance(await user.getAddress());

            const willingToPay = queryCost + 1n;
            const tx = await prover
                .connect(user)
                .submitQuery(sampleQuery, await user.getAddress(), { value: willingToPay });
            const receipt = await tx.wait();

            // amount held in escrow increased by amount specified by sender
            const escrowAfterSubmit = await prover.getTotalEscrowBalance();
            expect(escrowAfterSubmit).to.equal(willingToPay);

            // sender's funds decreased by gas fees + actual query cost
            const balanceAfterSubmit = await ethers.provider.getBalance(await user.getAddress());
            expect(balanceAfterSubmit).to.equal(
                // @ts-ignore
                balanceBeforeSubmit - willingToPay - receipt?.cumulativeGasUsed * receipt?.gasPrice,
            );

            const event = receipt?.logs[0];
            // @ts-ignore
            expect(event?.fragment.name).to.equal('QuerySubmitted');
        });

        it('Should reject query with insufficient payment', async function () {
            await expect(
                prover.connect(user).submitQuery(sampleQuery, await user.getAddress(), { value: queryCost - 1n }),
            ).to.be.revertedWithoutReason();
        });

        it('Should revert when query.chainId does not match chainId configured in contract', async function () {
            const bogusQuery = {
                chainId: 999,
                height: 1000n,
                index: 0,
                layoutSegments: [
                    { offset: 0, size: 32 },
                    { offset: 32, size: 64 },
                ],
            };

            await expect(
                prover.connect(user).submitQuery(bogusQuery, await user.getAddress(), { value: queryCost + 1n }),
            ).to.be.revertedWith('Chain not supported');
        });

        it('Should revert when a TimedOut query is submitted again', async function () {
            // submit query once
            const tx = await prover
                .connect(user)
                .submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n });
            const receipt = await tx.wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            // QueryState.TimedOut
            await prover.connect(owner).mock_setQueryState(queryId, 3);

            // submit the same query again
            await expect(
                prover.connect(user).submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n }),
            ).to.be.revertedWith('Query already timed out');
        });

        it('Should revert when an InvalidQuery query is submitted again', async function () {
            // submit query once
            const tx = await prover
                .connect(user)
                .submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n });
            const receipt = await tx.wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            // QueryState.InvalidQuery
            await prover.connect(owner).mock_setQueryState(queryId, 4);

            // submit the same query again
            await expect(
                prover.connect(user).submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n }),
            ).to.be.revertedWith('Query already invalidated');
        });

        it('Should revert when query is submitted again while still processing the first one', async function () {
            // submit query once
            const tx = await prover
                .connect(user)
                .submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n });
            const receipt = await tx.wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            // QueryState.Submitted
            await prover.connect(owner).mock_setQueryState(queryId, 1);

            // submit the same query again
            await expect(
                prover.connect(user).submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n }),
            ).to.be.revertedWith('Query already submitted, processing in progress');
        });
    });

    describe('reclaimEscrowedPayment()', function () {
        const queryStates = [
            { name: 'QueryState.TimedOut', value: 3 },
            { name: 'QueryState.InvalidQuery', value: 4 },
        ];

        queryStates.forEach(({ name, value }) => {
            it(`Should allow principal to reclaim escrow when ${name}`, async function () {
                const willingToPay = queryCost + 1n;
                const tx = await prover
                    .connect(user)
                    .submitQuery(sampleQuery, await user.getAddress(), { value: willingToPay });

                const receipt = await tx.wait();
                // @ts-ignore
                const queryId = receipt?.logs[0]?.args?.[0];

                if (!queryId) {
                    throw new Error('QueryId not found in event logs');
                }

                await prover.connect(owner).mock_setQueryState(queryId, value);
                const balanceBefore = await ethers.provider.getBalance(await user.getAddress());

                // this is held in escrow for now
                const escrowBeforeReclaim = await prover.getTotalEscrowBalance();
                expect(escrowBeforeReclaim).to.equal(willingToPay);

                const reclaimReceipt = await (await prover.connect(user).reclaimEscrowedPayment(queryId)).wait();
                const escrowAfterReclaim = await prover.getTotalEscrowBalance();

                // nothing held in escrow anymore
                expect(escrowAfterReclaim).to.eq(0);

                // sender's funds decreased by gas fees
                // actual query price held in escrow was restored
                const balanceAfter = await ethers.provider.getBalance(await user.getAddress());
                expect(balanceAfter).to.equal(
                    // @ts-ignore
                    // eslint-disable-next-line @typescript-eslint/restrict-plus-operands
                    balanceBefore - reclaimReceipt?.cumulativeGasUsed * reclaimReceipt?.gasPrice + willingToPay,
                );

                const event = reclaimReceipt?.logs[0];
                // @ts-ignore
                expect(event?.fragment.name).to.equal('EscrowedPaymentReclaimed');
            });
        });

        it(`Should not allow principal to reclaim escrow when query.state isn't supported`, async function () {
            const tx = await prover
                .connect(user)
                .submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n });
            const receipt = await tx.wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            // QueryState.Uninitialied
            await prover.connect(owner).mock_setQueryState(queryId, 0);

            await expect(prover.connect(user).reclaimEscrowedPayment(queryId)).to.be.revertedWith(
                'Query state does not allow reclaim',
            );
        });

        it('Should revert when sender != query.principal', async function () {
            const tx = await prover
                .connect(user)
                .submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n });
            const receipt = await tx.wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            // note: trying to reclaim as `owner` instead of `user`
            await expect(prover.connect(owner).reclaimEscrowedPayment(queryId)).to.be.revertedWith(
                'Sender different from query.principal',
            );
        });
    });

    describe('Query Proof Submission', function () {
        it('Should process valid proof submission', async function () {
            const tx = await prover
                .connect(user)
                .submitQuery(sampleQuery, await owner.getAddress(), { value: queryCost + 1n });

            const receipt = await tx.wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            const proof = new Uint8Array(32);
            // mark proof as valid
            await prover.mock_setVerifierResult(0);
            await prover.connect(owner).submitQueryProof(queryId, proof);

            const queryDetails = await prover.queries(queryId);
            // Query is verified and the result is available
            // aka QueryState.ResultAvailable,
            expect(queryDetails.state).to.equal(2);
        });

        it('Should only allow owner to submit proofs', async function () {
            const tx = await prover.submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n });

            const receipt = await tx.wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            const proof = new Uint8Array(32);

            await expect(prover.connect(user).submitQueryProof(queryId, proof)).to.be.revertedWith(
                'Caller is not the owner',
            );
        });
    });

    describe('Proceeds Withdrawal', function () {
        it('Should only allow owner to withdraw proceeds', async function () {
            await expect(prover.connect(user).withdrawProceeds()).to.be.revertedWith('Caller is not the owner');
        });
    });
});
