import { FrontierEvmEvent } from '@subql/frontier-evm-processor';
import { QueryVerification } from '../types';

// Event signatures for Native Query Verifier precompile
// QueryVerified(address indexed caller, bytes32 queryId, uint64 chainKey, uint64 height, uint8 status, (uint64,bytes32)[] resultSegments)
type QueryVerifiedArgs = [string, string, bigint, bigint, number, [bigint, string][]];

// QueryVerificationFailed(address indexed caller, bytes32 queryId, uint64 chainKey, uint64 height, uint8 status, string reason)
type QueryVerificationFailedArgs = [string, string, bigint, bigint, number, string];

export async function handleQueryVerified(event: FrontierEvmEvent<QueryVerifiedArgs>): Promise<void> {
    if (!event.args) {
        logger.error(`No args found for QueryVerified event`);
        return;
    }

    const [caller, queryId, chainKey, height, status, resultSegments] = event.args;

    logger.info(
        `Query verified: caller=${caller}, queryId=${queryId}, chainKey=${chainKey}, height=${height}, status=${status}`,
    );

    // Create a unique ID for this verification event
    const id = `${event.blockNumber}-${event.transactionIndex}-${event.logIndex || 0}`;

    // Parse the queryId to extract query details if possible
    // The queryId is a hash of the query parameters, so we store it as-is
    const verification = QueryVerification.create({
        id,
        caller: caller.toLowerCase(),
        queryId,
        chainId: BigInt(chainKey),
        height: BigInt(height),
        status,
        failureReason: undefined,
        blockNumber: BigInt(event.blockNumber),
        timestamp: event.blockTimestamp ? BigInt(event.blockTimestamp.getTime()) : BigInt(Date.now()),
        resultSegments: resultSegments
            ? resultSegments.map((segment) => ({
                  offset: segment[0].toString(),
                  bytes: segment[1],
              }))
            : [],
    });

    await verification.save();
}

export async function handleQueryVerificationFailed(
    event: FrontierEvmEvent<QueryVerificationFailedArgs>,
): Promise<void> {
    if (!event.args) {
        logger.error(`No args found for QueryVerificationFailed event`);
        return;
    }

    const [caller, queryId, chainKey, height, status, reason] = event.args;

    logger.info(
        `Query verification failed: caller=${caller}, queryId=${queryId}, chainKey=${chainKey}, height=${height}, status=${status}, reason=${reason}`,
    );

    // Create a unique ID for this verification event
    const id = `${event.blockNumber}-${event.transactionIndex}-${event.logIndex || 0}`;

    const verification = QueryVerification.create({
        id,
        caller: caller.toLowerCase(),
        queryId,
        chainId: BigInt(chainKey),
        height: BigInt(height),
        status,
        failureReason: reason,
        blockNumber: BigInt(event.blockNumber),
        timestamp: event.blockTimestamp ? BigInt(event.blockTimestamp.getTime()) : BigInt(Date.now()),
        resultSegments: [],
    });

    await verification.save();
}
