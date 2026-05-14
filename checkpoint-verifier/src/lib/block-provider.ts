import { JsonRpcProvider, WebSocketProvider, Block, TransactionReceipt } from 'ethers';

import { encoding } from '@gluwa/usc-sdk';

/**
 * Raw authorization data for EIP-7702 transactions.
 */
interface RawAuthorization {
    yParity: number;
}

/**
 * Raw transaction response with authorization list.
 */
class RawTransactionResponse {
    readonly authorizationList: RawAuthorization[] | null;

    constructor(authorizationList: RawAuthorization[] | null) {
        this.authorizationList = authorizationList;
    }
}

/**
 * Block data with receipts for merkle root computation.
 */
export interface BlockWithReceipts {
    block: Block;
    transactions: encoding.TransactionWithRaw[];
    receipts: TransactionReceipt[];
}

/**
 * Creates an ethers provider from a URL (HTTP or WebSocket).
 */
export function createProvider(rpcUrl: string): JsonRpcProvider | WebSocketProvider {
    if (rpcUrl.startsWith('ws://') || rpcUrl.startsWith('wss://')) {
        return new WebSocketProvider(rpcUrl);
    }
    return new JsonRpcProvider(rpcUrl);
}

/**
 * Fetches a block with all transaction receipts for merkle root computation.
 */
export async function getBlockWithReceipts(
    provider: JsonRpcProvider | WebSocketProvider,
    blockNumber: number,
): Promise<BlockWithReceipts | null> {
    // Small delay to avoid rate limiting
    await new Promise((resolve) => setTimeout(resolve, 50));

    // Fetch raw block data with transactions
    let blockDataRaw: Record<string, unknown>;
    try {
        blockDataRaw = (await provider.send('eth_getBlockByNumber', [`0x${blockNumber.toString(16)}`, true])) as Record<
            string,
            unknown
        >;

        if (!blockDataRaw) {
            console.error(`Block ${blockNumber} not found`);
            return null;
        }
    } catch (e) {
        console.error(`Error fetching block ${blockNumber}: ${(e as Error).message}`);
        return null;
    }

    // Get network for transaction formatting
    const network = await provider.getNetwork();

    // Wrap transactions into TransactionWithRaw objects
    const rawTransactions = blockDataRaw.transactions as Record<string, unknown>[];
    const transactions = rawTransactions.map((transaction) => {
        const formattedTx = (provider as JsonRpcProvider)._wrapTransactionResponse(transaction as never, network);

        // Map raw yParity values from JSON response
        const authList = transaction.authorizationList as { yParity: string }[] | undefined;
        const mappedList =
            authList?.map((auth) => ({
                yParity: Number(auth.yParity),
            })) || null;
        const rawTx = new RawTransactionResponse(mappedList);

        return new encoding.TransactionWithRaw(formattedTx, rawTx);
    });

    // Small delay before fetching receipts
    await new Promise((resolve) => setTimeout(resolve, 50));

    // Fetch receipts
    let receiptsRaw: Record<string, unknown>[];
    try {
        receiptsRaw = (await provider.send('eth_getBlockReceipts', [`0x${blockNumber.toString(16)}`])) as Record<
            string,
            unknown
        >[];

        if (!receiptsRaw) {
            console.error(`Receipts for block ${blockNumber} not found`);
            return null;
        }
    } catch (e) {
        console.error(`Error fetching receipts for block ${blockNumber}: ${(e as Error).message}`);
        return null;
    }

    // Wrap block and receipts
    const block = (provider as JsonRpcProvider)._wrapBlock(blockDataRaw as never, network);
    const receipts = receiptsRaw.map((r) => (provider as JsonRpcProvider)._wrapTransactionReceipt(r as never, network));

    return { block, transactions, receipts };
}

/**
 * Gets the latest block number from the provider.
 */
export async function getLatestBlockNumber(provider: JsonRpcProvider | WebSocketProvider): Promise<number> {
    return provider.getBlockNumber();
}

/**
 * Closes the provider connection if it's a WebSocket provider.
 */
export async function closeProvider(provider: JsonRpcProvider | WebSocketProvider): Promise<void> {
    if (provider instanceof WebSocketProvider) {
        await provider.destroy();
    }
}
