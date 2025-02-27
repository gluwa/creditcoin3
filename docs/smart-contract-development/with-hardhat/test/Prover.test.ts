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

    describe('Query Cost Computation', function () {
        it('Should compute correct query cost based on layout segments', function () {
            const expectedCost = 1000n + 96n * 10n;
            expect(queryCost).to.equal(expectedCost);
        });
    });

    describe('Query Submission', function () {
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
    });

    describe('Escrow Reclaim', function () {
        it('Should allow principal to reclaim escrow for timed out query', async function () {
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

            // QueryState.TimedOut
            await prover.connect(owner).mock_setQueryState(queryId, 3);
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
