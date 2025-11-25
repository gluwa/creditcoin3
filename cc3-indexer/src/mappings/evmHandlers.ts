import { FrontierEvmEvent } from '@subql/frontier-evm-processor';
import { TransactionVerified } from '../types';

// Event signature for Native Query Verifier precompile
// TransactionVerified(uint64 indexed chain_key, uint64 indexed height, uint8 txIndex)
// Note: chain_key and height are indexed (in topics), txIndex is in data
type TransactionVerifiedArgs = [bigint, bigint, number];

export async function handleTransactionVerified(event: FrontierEvmEvent<TransactionVerifiedArgs>): Promise<void> {
    if (!event.args) {
        logger.error(`No args found for TransactionVerified event`);
        return;
    }

    // Event structure: TransactionVerified(uint64 indexed chain_key, uint64 indexed height, uint8 txIndex)
    // Topics[0] = event signature hash
    // Topics[1] = chain_key (indexed)
    // Topics[2] = height (indexed)
    // Data = txIndex (uint8)
    const [chainKey, height, txIndex] = event.args;

    logger.info(`Transaction verified: chainKey=${chainKey}, height=${height}, txIndex=${txIndex}`);

    // Create a unique ID for this verification event
    const id = `${event.blockNumber}-${event.transactionIndex}-${event.logIndex || 0}`;

    // Store the verification event
    // The TransactionVerified event contains: chain_key, height, and txIndex
    const verification = TransactionVerified.create({
        id,
        chainId: BigInt(chainKey),
        height: BigInt(height),
        txIndex: txIndex, // Transaction index from the event
        blockNumber: BigInt(event.blockNumber),
        timestamp: event.blockTimestamp ? BigInt(event.blockTimestamp.getTime()) : BigInt(Date.now()),
    });

    await verification.save();
}
