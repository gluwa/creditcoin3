import { expect } from 'chai';
import { ethers } from 'hardhat';
import { Contract, Signer } from 'ethers';

describe('CreditcoinPublicProver', function () {
    let prover: Contract;
    let owner: Signer;
    let user: Signer;
    let proceedsAccount: Signer;
    let sampleQuery: any;

    before(async function () {
        [owner, user, proceedsAccount] = await ethers.getSigners();

        const CreditcoinPublicProver = await ethers.getContractFactory('CreditcoinPublicProver');
        prover = await CreditcoinPublicProver.deploy(await proceedsAccount.getAddress());

        sampleQuery = {
            chainId: 1,
            height: 1000n,
            index: 0,
            layoutSegments: [
                { offset: 0, size: 32 },
                { offset: 32, size: 64 },
            ],
        };
    });

    describe('Deployment', function () {
        it('Should set the right owner', async function () {
            expect(await prover.owner()).to.equal(await owner.getAddress());
        });

        it('Should initialize with zero total escrow balance', async function () {
            const totalEscrowBalance = await prover.totalEscrowBalance();
            expect(totalEscrowBalance).to.equal(0);
        });
    });

    describe('Query Cost Computation', function () {
        it('Should compute correct query cost based on layout segments', async function () {
            const cost = await prover.computeQueryCost(sampleQuery);
            const expectedCost = 1000n + 96n * 10n;
            expect(cost).to.equal(expectedCost);
        });
    });

    describe('Query Submission', function () {
        it('Should accept query with sufficient payment', async function () {
            const cost: bigint = await prover.computeQueryCost(sampleQuery);
            const tx = await prover
                .connect(user)
                .submitQuery(sampleQuery, await user.getAddress(), { value: cost + 1000n });

            const receipt = await tx.wait();
            const event = receipt?.logs[0];
            expect(event?.fragment.name).to.equal('QuerySubmitted');
        });

        it('Should reject query with insufficient payment', async function () {
            const cost = await prover.computeQueryCost(sampleQuery);
            await expect(
                prover.connect(user).submitQuery(sampleQuery, await user.getAddress(), { value: cost - 1n }),
            ).to.be.revertedWithoutReason();
        });
    });

    describe('Escrow Reclaim', function () {
        it('Should allow principal to reclaim escrow for timed out query', async function () {
            const cost: bigint = await prover.computeQueryCost(sampleQuery);
            const tx = await prover
                .connect(user)
                .submitQuery(sampleQuery, await user.getAddress(), { value: cost + 1n });

            const receipt = await tx.wait();
            const queryId = receipt?.logs[0]?.args?.[0];

            if (!queryId) {
                throw new Error('QueryId not found in event logs');
            }

            await prover.connect(owner).setQueryState(queryId);

            const balanceBefore = await ethers.provider.getBalance(await user.getAddress());
            await prover.connect(user).reclaimEscrowedPayment(queryId);
            const balanceAfter = await ethers.provider.getBalance(await user.getAddress());

            console.log('before: ', balanceBefore);
            console.log('after:  ', balanceAfter);
            expect(balanceAfter).to.be.gt(balanceBefore);
        });
    });

    describe('Query Proof Submission', function () {
        it('Should process valid proof submission', async function () {
            const cost: bigint = await prover.computeQueryCost(sampleQuery);
            const tx = await prover
                .connect(owner)
                .submitQuery(sampleQuery, await owner.getAddress(), { value: cost + 1000n });

            const receipt = await tx.wait();
            const queryId = receipt?.logs[0]?.args?.[0];

            const proof = new Uint8Array(32);
            await prover.connect(owner).submitQueryProof(queryId, proof);

            const queryDetails = await prover.queries(queryId);
            expect(queryDetails.state).to.equal(1);
        });

        it('Should only allow owner to submit proofs', async function () {
            const cost: bigint = await prover.computeQueryCost(sampleQuery);
            const tx = await prover.submitQuery(sampleQuery, await user.getAddress(), { value: cost + 1000n });

            const receipt = await tx.wait();
            const queryId = receipt?.logs[0]?.args?.[0];

            const proof = new Uint8Array(32);

            await expect(prover.connect(user).submitQueryProof(queryId, proof)).to.be.revertedWith(
                'Caller is not the owner',
            );
        });
    });

    describe('Proceeds Withdrawal', function () {
        it('Should allow owner to withdraw unescrow proceeds', async function () {
            await owner.sendTransaction({
                to: await prover.getAddress(),
                value: ethers.parseEther('1.0'),
            });

            const cost: bigint = await prover.computeQueryCost(sampleQuery);
            const tx = await prover
                .connect(owner)
                .submitQuery(sampleQuery, await owner.getAddress(), { value: cost + 1000n });

            const receipt = await tx.wait();
            const queryId = receipt?.logs[0]?.args?.[0];

            const proof = new Uint8Array(32);
            await prover.connect(owner).submitQueryProof(queryId, proof);

            const balanceBefore = await ethers.provider.getBalance(await proceedsAccount.getAddress());
            await prover.connect(owner).withdrawProceeds();
            const balanceAfter = await ethers.provider.getBalance(await proceedsAccount.getAddress());

            expect(balanceAfter).to.be.gt(balanceBefore);
        });

        it('Should only allow owner to withdraw proceeds', async function () {
            await expect(prover.connect(user).withdrawProceeds()).to.be.revertedWith('Caller is not the owner');
        });
    });
});
