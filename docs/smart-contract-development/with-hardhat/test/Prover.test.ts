import { expect } from 'chai';
import { ethers } from 'hardhat';
import { Signer, parseEther } from 'ethers';
import { ProverForTesting } from '../typechain-types';
import { progressBlocks } from './helpers';
import { time } from '@nomicfoundation/hardhat-toolbox/network-helpers';

const BLOCKTIME = 1; // 1 second per block

// see cli/src/lib/common.ts
const u8aToHex = (bytes: Uint8Array | Buffer): string => {
    const byteArray = Uint8Array.from(bytes);
    return byteArray.reduce((str, byte) => str + byte.toString(16).padStart(2, '0'), '0x');
};

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
    const TIMEOUT_BLOCKS = 10;

    // WARNING: using a high-level beforeEach() instead of before() b/c we want contract
    // to be redeployed for every test scenario so that each scenario starts in a fresh state!
    // Otherwise we'll have to deal with tracking internal contract state & removing queries
    beforeEach(async function () {
        [owner, user, proceedsAccount] = await ethers.getSigners();

        // NOTE: interacting with a contract that inherits the SUT b/c it exposes
        // additional helper methods, like mock_setQueryState() for example
        const proverFactory = await ethers.getContractFactory('ProverForTesting');
        prover = await proverFactory.deploy(
            await proceedsAccount.getAddress(),
            10n,
            1000n,
            sampleQuery.chainId,
            'testing',
            TIMEOUT_BLOCKS * BLOCKTIME,
        );
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
            await expect(prover.connect(owner).updateCostPerByte(100n))
                .to.emit(prover, 'CostPerByteUpdated')
                .withArgs(100n);

            const newCost = await prover.costPerByte();
            expect(newCost).to.equal(100n);

            // subsequent query cost calculation uses the new cost/byte
            const newQueryCost = await prover.computeQueryCost(sampleQuery);
            const expectedCost = 1000n + 96n * 100n;
            expect(newQueryCost).to.equal(expectedCost);
        });

        it('Does not allow calls from non-owner', async function () {
            await expect(prover.connect(user).updateCostPerByte(100n)).to.be.revertedWithCustomError(
                prover,
                'OwnableUnauthorizedAccount',
            );
        });
    });

    describe('updateBaseFee()', function () {
        it('Should store new fee and emit an event', async function () {
            const defaultFee = await prover.baseFee();
            // configured in the contract constructor
            expect(defaultFee).to.equal(1000n);

            // now let's change it
            await expect(prover.connect(owner).updateBaseFee(100n)).to.emit(prover, 'BaseFeeUpdated').withArgs(100n);

            const newFee = await prover.baseFee();
            expect(newFee).to.equal(100n);

            // subsequent query cost calculation uses the new fee
            const newQueryCost = await prover.computeQueryCost(sampleQuery);
            const expectedCost = 100n + 96n * 10n;
            expect(newQueryCost).to.equal(expectedCost);
        });

        it('Does not allow calls from non-owner', async function () {
            await expect(prover.connect(user).updateBaseFee(100n)).to.be.revertedWithCustomError(
                prover,
                'OwnableUnauthorizedAccount',
            );
        });
    });

    describe('Query Cost Computation', function () {
        it('Should compute correct query cost based on layout segments', function () {
            const expectedCost = 1000n + 96n * 10n;
            expect(queryCost).to.equal(expectedCost);
        });
    });

    describe('mock_submitQueryWithState()', function () {
        it('emits an event and updates with new state', async function () {
            // doesn't revert
            const receipt = await (
                await prover.connect(owner).mock_submitQueryWithState(
                    sampleQuery,
                    await user.getAddress(),
                    3, // QueryState.InvalidQuery
                    { value: queryCost },
                )
            ).wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            const event = receipt?.logs[0];
            // @ts-ignore
            expect(event?.fragment.name).to.equal('QuerySubmitted');

            // and the state is what we've specified above
            const queryDetails = await prover.queries(queryId);
            expect(queryDetails.state).to.equal(3);
        });
    });

    describe('submitQuery()', function () {
        it('Should accept query with sufficient payment', async function () {
            const escrowBeforeSubmit = await prover.getTotalEscrowBalance();
            expect(escrowBeforeSubmit).to.equal(0);

            const balanceBeforeSubmit = await ethers.provider.getBalance(await user.getAddress());

            const willingToPay = queryCost;
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
            ).to.be.revertedWith('Insufficient funds: msg.value must be >= estimatedCost');
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

        const queryStates = [
            { name: 'QueryState.Submitted', value: 1, revertMessage: 'Query already submitted and still pending' },
            {
                name: 'QueryState.ResultAvailable',
                value: 2,
                revertMessage: 'Query proof already generated, check contract storage for results',
            },
            { name: 'QueryState.InvalidQuery', value: 3, revertMessage: 'Cannot resubmit an invalid query' },
        ];
        queryStates.forEach(({ name, value, revertMessage }) => {
            it(`Should revert when ${name} query is submitted again`, async function () {
                // submit query once
                const tx = await prover
                    .connect(user)
                    .submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n });
                const receipt = await tx.wait();
                // @ts-ignore
                const queryId = receipt?.logs[0]?.args?.[0];

                // QueryState.TimedOut
                await prover.connect(owner).mock_setQueryState(queryId, value);

                // submit the same query again
                await expect(
                    prover.connect(user).submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n }),
                ).to.be.revertedWith(revertMessage);
            });
        });
    });

    describe('reclaimEscrowedPayment()', function () {
        const queryStates = [
            { name: 'QueryState.InvalidQuery', value: 3, timeout: 0 },
            { name: 'QueryState.InvalidQuery', value: 3, timeout: TIMEOUT_BLOCKS + 1 },
            { name: 'QueryState.Submitted', value: 1, timeout: TIMEOUT_BLOCKS + 1 },
        ];

        queryStates.forEach(({ name, value, timeout }) => {
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

                // Progress blocks
                await progressBlocks(timeout, BLOCKTIME);

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

                const queryDetails = await prover.queries(queryId);
                expect(queryDetails.escrowedAmount).to.equal(0);
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
                'Cannot reclaim: neither timeout nor invalid query state met',
            );
        });

        it(`Should not allow principal to reclaim escrow when query.state is ResultAvailable but timed out`, async function () {
            const tx = await prover
                .connect(user)
                .submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n });
            const receipt = await tx.wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            // QueryState.ResultAvailable
            await prover.connect(owner).mock_setQueryState(queryId, 2);

            // Progress blocks
            await progressBlocks(TIMEOUT_BLOCKS, BLOCKTIME);

            await expect(prover.connect(user).reclaimEscrowedPayment(queryId)).to.be.revertedWith(
                'Cannot reclaim: query result is available',
            );
        });

        it(`Should not allow principal to reclaim escrow when query isn't timed out`, async function () {
            const tx = await prover
                .connect(user)
                .submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n });

            const receipt = await tx.wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            // QueryState.Submitted
            await prover.connect(owner).mock_setQueryState(queryId, 1);

            const after = await time.latest();
            console.log(`after: ${after}`);

            await expect(prover.connect(user).reclaimEscrowedPayment(queryId)).to.be.revertedWith(
                'Cannot reclaim: neither timeout nor invalid query state met',
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

        it('Should allow principal to reclaim escrow when query has timed out', async function () {
            const receipt = await (
                await prover.connect(user).submitQuery(sampleQuery, await user.getAddress(), { value: queryCost })
            ).wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            const balanceBefore = await ethers.provider.getBalance(await user.getAddress());
            const escrowBeforeReclaim = await prover.getTotalEscrowBalance();

            // set state to QueryState.InvalidQuery
            await prover.connect(owner).mock_setQueryState(queryId, 3);
            // drain contract balance to cause a failure later
            await prover.connect(owner).mock_drainBalance(queryCost);

            await expect(prover.connect(user).reclaimEscrowedPayment(queryId)).to.be.revertedWithoutReason();

            const balanceAfter = await ethers.provider.getBalance(await user.getAddress());
            const escrowAfterReclaim = await prover.getTotalEscrowBalance();

            // b/c of gas fees paid
            expect(balanceAfter).to.be.below(balanceBefore);
            expect(escrowAfterReclaim).to.equal(escrowBeforeReclaim);

            const queryDetails = await prover.queries(queryId);
            expect(queryDetails.escrowedAmount).to.equal(queryCost);
        });
    });

    describe('submitQueryProof()', function () {
        const verificationResult = [{ result: 0, expectedState: 2, stateName: 'QueryState.ResultAvailable' }];

        verificationResult.forEach(({ result, expectedState, stateName }) => {
            it(`Should emit an event and set query.state to ${stateName} when verification result is ${result}`, async function () {
                const receipt = await (
                    await prover
                        .connect(user)
                        .submitQuery(sampleQuery, await owner.getAddress(), { value: queryCost + 1n })
                ).wait();

                // @ts-ignore
                const queryId = receipt?.logs[0]?.args?.[0];

                const queryDetailsBefore = await prover.queries(queryId);
                expect(queryDetailsBefore.state).to.equal(1); // QueryState.Submitted

                const proof = u8aToHex(new TextEncoder().encode(''));
                const expectedSegment = { offset: 1n, abiBytes: new Uint8Array(32) };
                // mark proof as invalid
                await prover.mock_setVerifierResult({
                    status: result,
                    resultSegments: [expectedSegment],
                });
                await expect(prover.connect(owner).submitQueryProof(queryId, proof))
                    .to.emit(prover, 'QueryProofVerified')
                    // note: not using expectedSegment b/c of serialization differences
                    .withArgs(queryId, [[1n, new Uint8Array(32)]], expectedState);

                // explicitly check query.state again
                const queryDetails = await prover.connect(user).getQueryDetails(queryId);
                expect(queryDetails.state).to.equal(expectedState);

                expect(queryDetails.resultSegments).to.have.lengthOf(1);
                const [offset, abiBytes] = queryDetails.resultSegments[0];
                expect(offset).to.equal(expectedSegment.offset);
                expect(abiBytes).to.equal('0x0000000000000000000000000000000000000000000000000000000000000000');
            });
        });

        it('Should revert when verifier.verify() reverts', async function () {
            const factory = await ethers.getContractFactory('ProverWhereVerifyReverts');
            const contract = await factory.deploy(
                await proceedsAccount.getAddress(),
                10n,
                1000n,
                sampleQuery.chainId,
                'testing',
                TIMEOUT_BLOCKS * BLOCKTIME,
            );
            await contract.waitForDeployment();

            const receipt = await (
                await contract.connect(user).submitQuery(sampleQuery, await user.getAddress(), { value: queryCost })
            ).wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            const proof = u8aToHex(new TextEncoder().encode(''));
            await expect(contract.connect(owner).submitQueryProof(queryId, proof)).to.be.revertedWith(
                'Reverted on purpose',
            );
        });

        it('Should only allow owner to submit proofs', async function () {
            const tx = await prover
                .connect(user)
                .submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n });

            const receipt = await tx.wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            const proof = u8aToHex(new TextEncoder().encode(''));

            await expect(prover.connect(user).submitQueryProof(queryId, proof)).to.be.revertedWithCustomError(
                prover,
                'OwnableUnauthorizedAccount',
            );
        });

        it('Should revert when proverFee transfer fails', async function () {
            const maxCost = (await ethers.provider.getBalance(await user.getAddress())) - parseEther('0.1');
            const receipt = await (
                await prover.connect(user).submitQuery(sampleQuery, await user.getAddress(), { value: maxCost })
            ).wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            let queryDetails = await prover.queries(queryId);
            expect(queryDetails.state).to.equal(1); // QueryState.Submitted

            // drain contract balance to cause a failure later
            await prover.connect(owner).mock_drainBalance(maxCost);

            const proof = u8aToHex(new TextEncoder().encode(''));
            await expect(prover.connect(owner).submitQueryProof(queryId, proof)).to.be.revertedWithoutReason();

            // explicitly check again that query.state did not change
            queryDetails = await prover.queries(queryId);
            expect(queryDetails.state).to.equal(1); // QueryState.Submitted
        });

        it('Should revert when query is timed out', async function () {
            const receipt = await (
                await prover.connect(user).submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n })
            ).wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            // Progress 10 seconds
            await progressBlocks(TIMEOUT_BLOCKS, BLOCKTIME);

            const proof = u8aToHex(new TextEncoder().encode(''));
            await expect(prover.connect(owner).submitQueryProof(queryId, proof)).to.be.revertedWith(
                'Query has timed out',
            );
        });
    });

    describe('Proceeds Withdrawal', function () {
        it('Should only allow owner to withdraw proceeds', async function () {
            await expect(prover.connect(user).withdrawProceeds()).to.be.revertedWithCustomError(
                prover,
                'OwnableUnauthorizedAccount',
            );
        });
    });

    describe('markAsInvalid()', function () {
        it('Should set query state to invalid', async function () {
            const tx = await prover
                .connect(user)
                .submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n });
            const receipt = await tx.wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            const oldQueryIds = await prover.allQueryIds();
            expect(oldQueryIds).to.include.members([queryId]);

            const oldQueryDetails = await prover.queries(queryId);
            expect(oldQueryDetails.query.chainId).to.equal(sampleQuery.chainId);
            expect(oldQueryDetails.query.height).to.equal(sampleQuery.height);
            expect(oldQueryDetails.query.index).to.equal(sampleQuery.index);

            // call
            await prover.connect(owner).markAsInvalid(queryId, 'Invalid query');

            const newQueryIds = await prover.allQueryIds();
            expect(newQueryIds).to.include.members([queryId]);

            // not removed, but state is set to InvalidQuery
            const newQueryDetails = await prover.queries(queryId);
            expect(newQueryDetails.query.chainId).to.equal(sampleQuery.chainId);
            expect(newQueryDetails.query.height).to.equal(sampleQuery.height);
            expect(newQueryDetails.query.index).to.equal(sampleQuery.index);
            expect(newQueryDetails.state).to.equal(3); // QueryState.InvalidQuery
        });

        it('Should set query state correctly when there are more than 1', async function () {
            const receiptOne = await (
                await prover.connect(user).submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n })
            ).wait();
            // @ts-ignore
            const queryIdOne = receiptOne?.logs[0]?.args?.[0];

            const queryTwo = sampleQuery;
            queryTwo.index = 444;
            const receiptTwo = await (
                await prover.connect(user).submitQuery(queryTwo, await user.getAddress(), { value: queryCost + 1n })
            ).wait();
            // @ts-ignore
            const queryIdTwo = receiptTwo?.logs[0]?.args?.[0];

            const oldQueryIds = await prover.allQueryIds();
            expect(oldQueryIds).to.include.members([queryIdOne, queryIdTwo]);

            // remove the 1st query to exercise an if branch inside FUT
            await prover.connect(owner).markAsInvalid(queryIdOne, 'Invalid query');

            const newQueryIds = await prover.allQueryIds();
            expect(newQueryIds).to.include.members([queryIdOne, queryIdTwo]);

            // this wasn't affected
            const qdTwo = await prover.queries(queryIdTwo);
            expect(qdTwo.state).to.equal(1); // QueryState.Submitted

            // this was set to InvalidQuery
            const qdOne = await prover.queries(queryIdOne);
            expect(qdOne.state).to.equal(3); // QueryState.InvalidQuery
        });

        it('Should repay escrowed amount when query is marked as invalid', async function () {
            const userAddress = await user.getAddress();

            const balanceBefore = await ethers.provider.getBalance(userAddress);

            const tx = await prover.connect(user).submitQuery(sampleQuery, userAddress, { value: queryCost + 1n });
            const receipt = await tx.wait();
            if (!receipt) {
                throw new Error('Transaction receipt was null');
            }

            const gasUsed = receipt.gasUsed * tx.gasPrice;

            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            const oldQueryDetails = await prover.queries(queryId);
            expect(oldQueryDetails.escrowedAmount).to.equal(queryCost + 1n);

            // owner pays this gas, not the user
            await expect(prover.connect(owner).markAsInvalid(queryId, 'Invalid query because of some reason'))
                .to.emit(prover, 'QueryProofVerificationFailed')
                .withArgs(queryId, 'Invalid query because of some reason');

            const newQueryDetails = await prover.queries(queryId);
            expect(newQueryDetails.escrowedAmount).to.equal(0n);

            const balanceAfter = await ethers.provider.getBalance(userAddress);

            // Expect only the submitQuery gas to be deducted
            // small tolerance for gas price fluctuations
            expect(balanceBefore - balanceAfter).to.be.closeTo(gasUsed, 2000);

            // Assert that contract totalEscrowedBalance was decreased by exactly the cost paid for this query
            const totalEscrowedBalanceAfter = await prover.getTotalEscrowBalance();
            expect(totalEscrowedBalanceAfter).to.equal(0n);
        });

        it('Should NOT remove when ID does not match', async function () {
            const tx = await prover
                .connect(user)
                .submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n });
            const receipt = await tx.wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            const oldQueryIds = await prover.allQueryIds();
            expect(oldQueryIds).to.include.members([queryId]);

            const oldQueryDetails = await prover.queries(queryId);
            expect(oldQueryDetails.query.chainId).to.equal(sampleQuery.chainId);
            expect(oldQueryDetails.query.height).to.equal(sampleQuery.height);
            expect(oldQueryDetails.query.index).to.equal(sampleQuery.index);

            // call with an id which doesn't match
            // Should revert and not change anything
            await expect(
                prover
                    .connect(owner)
                    .markAsInvalid(
                        '0x9999999999999999999999999999999999999999999999999999999999999999',
                        'Invalid query',
                    ),
            ).to.be.revertedWith('Query not found');

            const newQueryIds = await prover.allQueryIds();
            expect(newQueryIds).to.include.members([queryId]);

            // nothing has changed
            const newQueryDetails = await prover.queries(queryId);
            expect(newQueryDetails.query.chainId).to.equal(sampleQuery.chainId);
            expect(newQueryDetails.query.height).to.equal(sampleQuery.height);
            expect(newQueryDetails.query.index).to.equal(sampleQuery.index);
        });

        it('Does not allow calls from non-owner', async function () {
            const tx = await prover
                .connect(user)
                .submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n });
            const receipt = await tx.wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            await expect(prover.connect(user).markAsInvalid(queryId, 'Invalid query')).to.be.revertedWithCustomError(
                prover,
                'OwnableUnauthorizedAccount',
            );
        });

        it('Should revert when trying to mark as invalid with result available', async function () {
            const tx = await prover
                .connect(user)
                .submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n });
            const receipt = await tx.wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            // Set state to ResultAvailable
            await prover.connect(owner).mock_setQueryState(queryId, 2);

            await expect(prover.connect(owner).markAsInvalid(queryId, 'Invalid query')).to.be.revertedWith(
                'Cannot mark as invalid: result available',
            );
        });
    });

    describe('getUnprocessedQueries()', function () {
        it('starts with zero queries', async function () {
            const unprocessed = await prover.connect(user).getUnprocessedQueries();
            // eslint-disable-next-line
            expect(unprocessed).to.be.empty;
        });

        it('submitted query is immediately reported as unprocessed', async function () {
            const tx = await prover
                .connect(user)
                .submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n });
            const receipt = await tx.wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];
            const queryDetails = await prover.queries(queryId);

            const unprocessed = await prover.connect(user).getUnprocessedQueries();
            expect(unprocessed.length).to.equal(1);
            expect(unprocessed[0].chainId).to.equal(queryDetails.query.chainId);
            expect(unprocessed[0].height).to.equal(queryDetails.query.height);
            expect(unprocessed[0].index).to.equal(queryDetails.query.index);
        });

        it('processed queries are not returned', async function () {
            const receiptOne = await (
                await prover.connect(user).submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n })
            ).wait();
            // @ts-ignore
            const queryIdOne = receiptOne?.logs[0]?.args?.[0];

            const queryTwo = sampleQuery;
            queryTwo.height = 4n;
            queryTwo.index = 444;
            const receiptTwo = await (
                await prover.connect(user).submitQuery(queryTwo, await user.getAddress(), { value: queryCost + 1n })
            ).wait();
            // @ts-ignore
            const queryIdTwo = receiptTwo?.logs[0]?.args?.[0];
            const qdTwo = await prover.queries(queryIdTwo);

            // QueryState.ResultAvailable
            await prover.connect(owner).mock_setQueryState(queryIdOne, 2);

            const unprocessed = await prover.connect(user).getUnprocessedQueries();
            expect(unprocessed.length).to.equal(1);
            expect(unprocessed[0].chainId).to.equal(qdTwo.query.chainId);
            expect(unprocessed[0].height).to.equal(qdTwo.query.height);
            expect(unprocessed[0].index).to.equal(qdTwo.query.index);
        });

        it('timedout queries are not returned', async function () {
            await (
                await prover.connect(user).submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n })
            ).wait();

            // Progress blocks
            await progressBlocks(TIMEOUT_BLOCKS, BLOCKTIME);

            const unprocessed = await prover.connect(user).getUnprocessedQueries();
            // Still one unprocessed query
            expect(unprocessed.length).to.equal(1);
        });
    });

    describe('getQueryDetails(), result segments', function () {
        it('Should revert when state is QueryState.Uninitialized', async function () {
            const receipt = await (
                await prover.connect(user).submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n })
            ).wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            // explicitly set the state: QueryState.Uninitialized
            await prover.connect(owner).mock_setQueryState(queryId, 0);

            await expect(prover.connect(user).getQueryDetails(queryId)).to.be.revertedWith('No such query');
        });

        it('Should return query details', async function () {
            const receipt = await (
                await prover.connect(user).submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n })
            ).wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            // explicitly set the state: QueryState.ResultAvailable
            await prover.connect(owner).mock_setQueryState(queryId, 2);
            const expectedSegment = { offset: 1n, abiBytes: new Uint8Array(32) };
            // mock queryDetails just so we have result segments to look for later
            await (
                await prover.connect(owner).mock_pushQueryDetails(
                    // QueryId 0
                    queryId,
                    // QueryDetails
                    {
                        // Result available
                        state: 2,
                        query: {
                            chainId: 1,
                            height: 12345678,
                            index: 0,
                            layoutSegments: [],
                        },
                        escrowedAmount: '1000000000000000000',
                        principal: '0x1234567890abcdef1234567890abcdef12345678',
                        estimatedCost: '25000000000000000',
                        timestamp: '0',
                        resultSegments: [expectedSegment],
                    },
                )
            ).wait();
            // doesn't crash
            const result = await prover.connect(user).getQueryDetails(queryId);
            expect(result.state).to.equal(2); // QueryState.ResultAvailable

            const resultSegments = result.resultSegments;
            expect(resultSegments).to.have.lengthOf(1);

            const [offset, abiBytes] = resultSegments[0];
            expect(offset).to.equal(expectedSegment.offset);
            expect(abiBytes).to.equal('0x0000000000000000000000000000000000000000000000000000000000000000');
        });
    });

    describe('withdrawProceeds()', function () {
        it('Does not allow calls from non-owner', async function () {
            await expect(prover.connect(user).withdrawProceeds()).to.be.revertedWithCustomError(
                prover,
                'OwnableUnauthorizedAccount',
            );
        });

        it('Does not update balance when contract balance is zero', async function () {
            // drain contract balance to cause a failure later
            let contractBalance = await ethers.provider.getBalance(await prover.getAddress());
            await prover.connect(owner).mock_drainBalance(contractBalance);
            contractBalance = await ethers.provider.getBalance(await prover.getAddress());
            expect(contractBalance).to.equal(0);

            const ownerBalanceBefore = await ethers.provider.getBalance(await owner.getAddress());
            const proceedsBalanceBefore = await ethers.provider.getBalance(await proceedsAccount.getAddress());

            await expect(prover.connect(owner).withdrawProceeds()).to.be.revertedWith(
                'No withdrawable proceeds available',
            );

            const ownerBalanceAfter = await ethers.provider.getBalance(await owner.getAddress());
            const proceedsBalanceAfter = await ethers.provider.getBalance(await proceedsAccount.getAddress());

            expect(ownerBalanceAfter).to.be.below(ownerBalanceBefore); // b/c of gas fees
            expect(proceedsBalanceAfter).to.equal(proceedsBalanceBefore);
        });

        it('Updates proceedsAccount balance when contract balance greater than zero', async function () {
            // setup
            const howMuch = parseEther('0.001');
            await prover.connect(user).mock_addBalance({
                value: howMuch,
            });
            const contractBalanceBefore = await ethers.provider.getBalance(await prover.getAddress());
            expect(contractBalanceBefore).to.be.above(0);

            const ownerBalanceBefore = await ethers.provider.getBalance(await owner.getAddress());
            const proceedsBalanceBefore = await ethers.provider.getBalance(await proceedsAccount.getAddress());

            // call FUT
            await expect(prover.connect(owner).withdrawProceeds())
                .to.emit(prover, 'ProceedsWithdrawn')
                .withArgs(await proceedsAccount.getAddress(), howMuch);

            const contractBalanceAfter = await ethers.provider.getBalance(await prover.getAddress());
            expect(contractBalanceAfter).to.equal(0);

            const ownerBalanceAfter = await ethers.provider.getBalance(await owner.getAddress());
            const proceedsBalanceAfter = await ethers.provider.getBalance(await proceedsAccount.getAddress());

            expect(ownerBalanceAfter).to.be.below(ownerBalanceBefore); // b/c of gas fees
            expect(proceedsBalanceAfter).to.be.above(proceedsBalanceBefore);
            expect(proceedsBalanceAfter).to.equal(proceedsBalanceBefore + contractBalanceBefore);
        });
    });

    describe('getQueryResult()', function () {
        it('Should reject query with mismatched chainId', async function () {
            const queryTwo = sampleQuery;
            queryTwo.chainId = 999999;

            await expect(prover.connect(user).getQueryResult(queryTwo)).to.be.revertedWith('Chain not supported');
        });

        it('Should return empty result segment when query state != QueryState.ResultAvailable', async function () {
            await prover.connect(owner).mock_submitQueryWithState(
                sampleQuery,
                await user.getAddress(),
                3, // QueryState.InvalidQuery
                { value: queryCost },
            );

            const resultSegments = await prover.connect(user).getQueryResult(sampleQuery);
            expect(resultSegments).to.have.lengthOf(0);
        });

        it('Should return result segments when query state == QueryState.ResultAvailable', async function () {
            const receipt = await (
                await prover.connect(user).submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n })
            ).wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            // explicitly set the state: QueryState.ResultAvailable
            await prover.connect(owner).mock_setQueryState(queryId, 2);
            const expectedSegment = { offset: 1n, abiBytes: new Uint8Array(32) };
            // mock queryDetails just so we have result segments to look for later
            await (
                await prover.connect(owner).mock_pushQueryDetails(
                    // QueryId 0
                    queryId,
                    // QueryDetails
                    {
                        // ResultAvailable
                        state: 2,
                        query: {
                            chainId: 1,
                            height: 12345678,
                            index: 0,
                            layoutSegments: [],
                        },
                        escrowedAmount: '1000000000000000000',
                        principal: '0x1234567890abcdef1234567890abcdef12345678',
                        estimatedCost: '25000000000000000',
                        timestamp: '0',
                        resultSegments: [expectedSegment],
                    },
                )
            ).wait();

            const resultSegments = await prover.connect(user).getQueryResult(sampleQuery);
            expect(resultSegments).to.have.lengthOf(1);

            const [offset, abiBytes] = resultSegments[0];
            expect(offset).to.equal(expectedSegment.offset);
            expect(abiBytes).to.equal('0x0000000000000000000000000000000000000000000000000000000000000000');
        });
    });
});
