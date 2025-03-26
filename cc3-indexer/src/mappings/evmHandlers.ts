import { FrontierEvmEvent } from '@subql/frontier-evm-processor';
import { BigNumber } from 'ethers';

import { Prover, ChainQueries, Proof, EscrowPaymentReclaimed, ProceedsWithdrawn, QueryStatus } from '../types';
import {
    QuerySubmittedEventObject,
    ChainQueryStructOutput,
    ResultSegmentStructOutput,
    ProverDeployedEventObject,
    EscrowedPaymentReclaimedEventObject,
    ProceedsWithdrawnEventObject,
    QueryProofVerifiedEventObject,
    BaseFeeUpdatedEventObject,
    CostPerByteUpdatedEventObject,
} from '../types/chain/ProverAbi';

// event ProverDeployed(address indexed contractAddress, address indexed owner, address proceedsAccount);
type ProverDeployedArgs = [string, string, string, bigint, bigint, bigint, string, bigint] & ProverDeployedEventObject;

export async function handleProverDeployed(event: FrontierEvmEvent<ProverDeployedArgs>): Promise<void> {
    if (!event.args) {
        logger.error(`No args found for ProverDeployed event`);
        return;
    }

    const [contractAddress, owner, proceedsAccount, baseCostPerByte, baseFee, chainKey, name, timeout] = event.args;

    const id = `${event.blockNumber} - ${event.transactionIndex}`;

    logger.info(
        `Prover deployed: ${id} ${owner} ${proceedsAccount} ${contractAddress}, chain: ${chainKey}, name: ${name}`,
    );

    const prover = Prover.create({
        id,
        owner,
        proceedsAccount,
        contractAddress,
        baseCostPerByte: BigInt(baseCostPerByte.toString()),
        baseFee: BigInt(baseFee.toString()),
        chainKey: BigInt(chainKey.toString()),
        name,
        timeout: BigInt(timeout.toString()),
    });

    await prover.save();
}

type QuerySubmittedArgs = [string, BigNumber, BigNumber, ChainQueryStructOutput] & QuerySubmittedEventObject;

export async function handleQuerySubmitted(event: FrontierEvmEvent<QuerySubmittedArgs>): Promise<void> {
    if (!event.args) {
        logger.error(`No args found for QuerySubmitted event`);
        return;
    }

    const [queryId, estimatedCost, escrowedAmount, chainQuery] = event.args;

    logger.info(`Query with ID ${queryId} subbmitted`);

    const id = `query-${event.blockNumber}-${event.transactionIndex}`;

    const queryEntity = ChainQueries.create({
        id,
        chainQueryId: queryId.toString(),
        chainKey: chainQuery.chainId.toNumber(),
        height: chainQuery.height.toBigInt(),
        index: chainQuery.index.toBigInt(),
        layoutSegments: chainQuery.layoutSegments.map((segment) => {
            return {
                offset: segment.offset.toBigInt(),
                size: segment.size.toBigInt(),
            };
        }),
        state: QueryStatus.Submitted,
        estimatedCost: estimatedCost.toBigInt(),
        escrowedAmount: escrowedAmount.toBigInt(),
    });

    await queryEntity.save();
}

// event QueryProofVerified(QueryId indexed queryId, bytes proof);
type QueryProofVerifiedArgs = [string, ResultSegmentStructOutput, number] & QueryProofVerifiedEventObject;

export async function handleQueryProofVerified(event: FrontierEvmEvent<QueryProofVerifiedArgs>): Promise<void> {
    if (!event.args) {
        logger.error(`No args found for QueryProofVerified event`);
        return;
    }

    const { queryId, resultSegments, state } = event.args;

    logger.info(`Query proof verified for query ${queryId}, state: ${state}`);

    const proofEntity = Proof.create({
        id: `${event.blockNumber}-${event.transactionIndex}`,
        queryRef: queryId,
        resultSegments: resultSegments.map((segment) => {
            return {
                offset: segment.offset.toBigInt(),
                bytes: segment.abiBytes.toString(),
            };
        }),
    });

    await proofEntity.save();

    // Get the query entity
    const queries = await ChainQueries.getByFields([['chainQueryId', '=', queryId]], { limit: 1 });
    if (queries.length === 0) {
        logger.error(`Query with ID ${queryId} not found`);
        return;
    }
    const query = queries[0];

    // Update state
    switch (state) {
        case 0:
            query.state = QueryStatus.Uninitialized;
            break;
        case 1:
            query.state = QueryStatus.Submitted;
            break;
        case 2:
            query.state = QueryStatus.ResultAvailable;
            break;
        case 3:
            query.state = QueryStatus.InvalidQuery;
            break;
        default:
            query.state = QueryStatus.InvalidQuery;
    }

    // Save the updated query
    await query.save();
}

// event EscrowedPaymentReclaimed(QueryId indexed queryId, uint256 escrowedAmount);
type EscrowedPaymentReclaimedArgs = [string, BigNumber] & EscrowedPaymentReclaimedEventObject;

export async function handleEscrowedPaymentReclaimed(
    event: FrontierEvmEvent<EscrowedPaymentReclaimedArgs>,
): Promise<void> {
    if (!event.args) {
        logger.error(`No args found for EscrowedPaymentReclaimed event`);
        return;
    }

    const { queryId, escrowedAmount } = event.args;

    logger.info(`Escrowed payment reclaimed for query ${queryId}`);

    const reclaimedPayment = EscrowPaymentReclaimed.create({
        id: `${event.blockNumber}-${event.transactionIndex}`,
        blockNumber: event.blockNumber,
        who: event.address,
        amount: escrowedAmount.toBigInt(),
    });

    await reclaimedPayment.save();
}

// event ProceedsWithdrawn(address indexed proceedsAccount, uint256 amount);
type ProceedsWithdrawnArgs = [string, BigNumber] & ProceedsWithdrawnEventObject;

export async function handleProceedsWithdrawn(event: FrontierEvmEvent<ProceedsWithdrawnArgs>): Promise<void> {
    if (!event.args) {
        logger.error(`No args found for ProceedsWithdrawn event`);
        return;
    }

    const { proceedsAccount, amount } = event.args;

    logger.info(`Proceeds withdrawn by ${proceedsAccount}`);

    const withdrawn = ProceedsWithdrawn.create({
        id: `${event.blockNumber}-${event.transactionIndex}`,
        blockNumber: event.blockNumber,
        who: event.address,
        proceedsAccount,
        amount: amount.toBigInt(),
    });

    await withdrawn.save();
}

type BaseFeeUpdatedArgs = [BigNumber] & BaseFeeUpdatedEventObject;

export async function handleUpdateBaseFee(event: FrontierEvmEvent<BaseFeeUpdatedArgs>): Promise<void> {
    if (!event.args) {
        logger.error(`No args found for BaseFeeUpdated event`);
        return;
    }

    const proverContractAddress = event.address;

    const [newBaseFee] = event.args;

    logger.info(`Base fee updated to ${newBaseFee.toString()}`);

    // Update the prover entity
    const provers = await Prover.getByFields([['contractAddress', '=', proverContractAddress]], { limit: 1 });
    for (const prover of provers) {
        prover.baseFee = BigInt(newBaseFee.toString());
        await prover.save();
    }
}

type CostPerByteUpdatedArgs = [BigNumber] & CostPerByteUpdatedEventObject;

export async function handleUpdateCostPerByte(event: FrontierEvmEvent<CostPerByteUpdatedArgs>): Promise<void> {
    if (!event.args) {
        logger.error(`No args found for CostPerByteUpdated event`);
        return;
    }

    const proverContractAddress = event.address;

    const [newCostPerByte] = event.args;

    logger.info(`Cost per byte updated to ${newCostPerByte.toString()}`);

    // Update the prover entity
    const provers = await Prover.getByFields([['contractAddress', '=', proverContractAddress]], { limit: 1 });
    for (const prover of provers) {
        prover.baseCostPerByte = BigInt(newCostPerByte.toString());
        await prover.save();
    }
}
