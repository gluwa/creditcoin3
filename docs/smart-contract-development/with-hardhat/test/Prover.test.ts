import { expect } from 'chai';
import { ethers } from 'hardhat';
import { Signer, parseEther } from 'ethers';
import { ProverForTesting } from '../typechain-types';
import { progressBlocks } from './helpers';
import { time } from '@nomicfoundation/hardhat-toolbox/network-helpers';

const BLOCKTIME = 1; // 1 second per block

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
            await expect(prover.connect(user).updateCostPerByte(100n)).to.be.revertedWith('Caller is not the owner');
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
            await expect(prover.connect(user).updateBaseFee(100n)).to.be.revertedWith('Caller is not the owner');
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
            { name: 'QueryState.Submitted', value: 1 },
            { name: 'QueryState.ResultAvailable', value: 2 },
            { name: 'QueryState.InvalidQuery', value: 3 },
        ];
        queryStates.forEach(({ name, value }) => {
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
                ).to.be.revertedWith('Query already exists');
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
        const verificationResult = [
            { result: 0, expectedState: 2, stateName: 'QueryState.ResultAvailable' },
            { result: 1, expectedState: 3, stateName: 'QueryState.InvalidQuery' },
            { result: 2, expectedState: 3, stateName: 'QueryState.InvalidQuery' },
            { result: 3, expectedState: 3, stateName: 'QueryState.InvalidQuery' },
            { result: 4, expectedState: 3, stateName: 'QueryState.InvalidQuery' },
        ];

        verificationResult.forEach(({ result, expectedState, stateName }) => {
            it(`Should emit an event and set query.state to ${stateName} when verification result is ${result}`, async function () {
                const receipt = await (
                    await prover
                        .connect(user)
                        .submitQuery(sampleQuery, await owner.getAddress(), { value: queryCost + 1n })
                ).wait();

                // @ts-ignore
                const queryId = receipt?.logs[0]?.args?.[0];

                let queryDetails = await prover.queries(queryId);
                expect(queryDetails.state).to.equal(1); // QueryState.Submitted

                const proof = new Uint8Array(32);
                // mark proof as invalid
                await prover.mock_setVerifierResult(result);
                await expect(prover.connect(owner).submitQueryProof(queryId, proof))
                    .to.emit(prover, 'QueryProofVerified')
                    .withArgs(queryId, [], expectedState);

                // explicitly check query.state again
                queryDetails = await prover.queries(queryId);
                expect(queryDetails.state).to.equal(expectedState);
            });
        });

        it('Should only allow owner to submit proofs', async function () {
            const tx = await prover
                .connect(user)
                .submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n });

            const receipt = await tx.wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            const proof = new Uint8Array(32);

            await expect(prover.connect(user).submitQueryProof(queryId, proof)).to.be.revertedWith(
                'Caller is not the owner',
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

            const proof = new Uint8Array(32);
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

            const proof = new Uint8Array(32);
            await expect(prover.connect(owner).submitQueryProof(queryId, proof)).to.be.revertedWith(
                'Query has timed out',
            );
        });
    });

    describe('Proceeds Withdrawal', function () {
        it('Should only allow owner to withdraw proceeds', async function () {
            await expect(prover.connect(user).withdrawProceeds()).to.be.revertedWith('Caller is not the owner');
        });
    });

    describe('removeQueryId()', function () {
        it('Should remove queries from internal storage', async function () {
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
            await prover.connect(owner).removeQueryId(queryId);

            const newQueryIds = await prover.allQueryIds();
            expect(newQueryIds).to.not.include.members([queryId]);
            // eslint-disable-next-line
            expect(newQueryIds).to.be.empty;

            // not removed, but storage is zeroed out
            const newQueryDetails = await prover.queries(queryId);
            expect(newQueryDetails.query.chainId).to.equal(0n);
            expect(newQueryDetails.query.height).to.equal(0n);
            expect(newQueryDetails.query.index).to.equal(0n);
        });

        it('Should remove queries when there are more than 1', async function () {
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
            await prover.connect(owner).removeQueryId(queryIdOne);

            const newQueryIds = await prover.allQueryIds();
            expect(newQueryIds).to.not.include.members([queryIdOne]);
            expect(newQueryIds).to.include.members([queryIdTwo]);

            // not removed, but storage is zeroed out
            const qdOne = await prover.queries(queryIdOne);
            expect(qdOne.query.chainId).to.equal(0n);
            expect(qdOne.query.height).to.equal(0n);
            expect(qdOne.query.index).to.equal(0n);

            // this wasn't affected
            const qdTwo = await prover.queries(queryIdTwo);
            expect(qdTwo.query.chainId).to.equal(queryTwo.chainId);
            expect(qdTwo.query.height).to.equal(queryTwo.height);
            expect(qdTwo.query.index).to.equal(queryTwo.index);
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
            // there should be no error and query should not be removed
            await prover
                .connect(owner)
                .removeQueryId('0x9999999999999999999999999999999999999999999999999999999999999999');

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

            await expect(prover.connect(user).removeQueryId(queryId)).to.be.revertedWith('Caller is not the owner');
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

            // QueryState.TimedOut
            await prover.connect(owner).mock_setQueryState(queryIdOne, 3);

            const unprocessed = await prover.connect(user).getUnprocessedQueries();
            expect(unprocessed.length).to.equal(1);
            expect(unprocessed[0].chainId).to.equal(qdTwo.query.chainId);
            expect(unprocessed[0].height).to.equal(qdTwo.query.height);
            expect(unprocessed[0].index).to.equal(qdTwo.query.index);
        });
    });

    describe('getQueryResultSegments()', function () {
        it('Should revert when query result not available yet', async function () {
            const receipt = await (
                await prover.connect(user).submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n })
            ).wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            // explicitly set the state: QueryState.TimedOut
            await prover.connect(owner).mock_setQueryState(queryId, 3);

            await expect(prover.connect(user).getQueryResultSegments(queryId)).to.be.revertedWith(
                'Query result not available',
            );
        });

        it('Should return query result segments', async function () {
            const receipt = await (
                await prover.connect(user).submitQuery(sampleQuery, await user.getAddress(), { value: queryCost + 1n })
            ).wait();
            // @ts-ignore
            const queryId = receipt?.logs[0]?.args?.[0];

            // explicitly set the state: QueryState.ResultAvailable
            await prover.connect(owner).mock_setQueryState(queryId, 2);
            const expectedSegment = { offset: 1n, abiBytes: new Uint8Array(32) };
            await prover.connect(owner).mock_pushQueryResultSegment(expectedSegment);

            // doesn't crash
            const result = await prover.connect(user).getQueryResultSegments(queryId);
            expect(result).to.have.lengthOf(1);

            const [offset, abiBytes] = result[0];
            expect(offset).to.equal(expectedSegment.offset);
            expect(abiBytes).to.equal('0x0000000000000000000000000000000000000000000000000000000000000000');
        });

        it('Should revert when verifier.get_query_result_segments() reverts', async function () {
            const factory = await ethers.getContractFactory('ProverWhereVerifierGetResultSegmentsReverts');
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

            // explicitly set the state: QueryState.ResultAvailable
            await contract.connect(owner).mock_setQueryState(queryId, 2);

            await expect(contract.connect(user).getQueryResultSegments(queryId)).to.be.revertedWith(
                'Reverted on purpose',
            );
        });

        it('Should revert when verifier.get_query_result_segments() errors', async function () {
            const factory = await ethers.getContractFactory('ProverWhereVerifierGetResultSegmentsErrors');
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

            // explicitly set the state: QueryState.ResultAvailable
            await contract.connect(owner).mock_setQueryState(queryId, 2);

            await expect(contract.connect(user).getQueryResultSegments(queryId)).to.be.revertedWith(
                'Errored on purpose',
            );
        });
    });

    describe('withdrawProceeds()', function () {
        it('Does not allow calls from non-owner', async function () {
            await expect(prover.connect(user).withdrawProceeds()).to.be.revertedWith('Caller is not the owner');
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
});
