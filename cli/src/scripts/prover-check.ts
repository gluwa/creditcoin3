import { mkdirSync, writeFileSync } from 'fs';
import { blockProver, chainInfo, proofProvider, utils } from '@gluwa/usc-sdk';
import EvmV1DecoderABI from '@gluwa/usc-sdk/dist/utils/evmV1DecoderAbi.json';
import ChainInfoABI from '@gluwa/usc-sdk/dist/chain-info/chain_info.json';
import { Contract, JsonRpcProvider, Wallet, WebSocketProvider } from 'ethers';
import { createClient } from 'graphqurl';
import axios from 'axios';

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

const graphQLQuery = (serverUrl: string, queryString: string) => {
    return createClient({
        endpoint: serverUrl,
        headers: {
            /* eslint-disable @typescript-eslint/naming-convention */
            'content-type': 'application/json',
        },
    }).query({
        query: queryString,
    });
};

function writeToDisk(dirPath: string, proofData: any) {
    const clonedData = { ...proofData };
    // reset variable fields so they don't choke git-diff later
    clonedData.chainKey = 0;
    clonedData.generatedAt = 'reset-on-purpose';

    mkdirSync(`${dirPath}/${clonedData.headerNumber}`, { recursive: true });

    writeFileSync(
        `${dirPath}/${clonedData.headerNumber}/${clonedData.txHash}.txt`,
        JSON.stringify(clonedData, null, 2) + '\n',
        {
            flag: 'w',
        },
    );
}

function toRpcBlockNumber(value: any) {
    if (!value || value === 'latest') return 'latest';
    return `0x${BigInt(value).toString(16)}`;
}

async function sampleNormalTransaction(provider: JsonRpcProvider, blockNumber: any = 'latest') {
    // `true` requests full transaction objects instead of only hashes.
    const block = await provider.send('eth_getBlockByNumber', [toRpcBlockNumber(blockNumber), true]);

    if (!block) {
        throw new Error(`Block ${blockNumber} was not found`);
    }

    const transactions = block.transactions.filter((tx: any) => typeof tx === 'object' && tx.gas != null);

    if (transactions.length === 0) {
        throw new Error('Block contains no transactions');
    }

    const sortedGasLimits = transactions
        .map((tx: any) => BigInt(tx.gas))
        .sort((a: bigint, b: bigint) => (a < b ? -1 : a > b ? 1 : 0));

    // Remove the lowest and highest 10% so large deployments and unusual
    // transactions do not distort the definition of a "normal" transaction.
    const trim = Math.floor(sortedGasLimits.length * 0.1);
    const normalRange =
        sortedGasLimits.length >= 10 ? sortedGasLimits.slice(trim, sortedGasLimits.length - trim) : sortedGasLimits;

    const averageGasLimit =
        normalRange.reduce((sum: bigint, gas: bigint) => sum + gas, 0n) / BigInt(normalRange.length);

    const sampledTransaction = transactions.reduce((closest: any, tx: any) => {
        const gas = BigInt(tx.gas);
        const closestGas = BigInt(closest.gas);

        const distance = gas >= averageGasLimit ? gas - averageGasLimit : averageGasLimit - gas;

        const closestDistance =
            closestGas >= averageGasLimit ? closestGas - averageGasLimit : averageGasLimit - closestGas;

        return distance < closestDistance ? tx : closest;
    });

    return {
        blockNumber: Number(BigInt(block.number)),
        transactionCount: transactions.length,
        normalAverageGasLimit: averageGasLimit.toString(),
        transaction: {
            hash: sampledTransaction.hash,
            from: sampledTransaction.from,
            to: sampledTransaction.to,
            gasLimit: BigInt(sampledTransaction.gas).toString(),
            type: Number(BigInt(sampledTransaction.type ?? '0x0')),
            inputBytes: Math.max(0, (sampledTransaction.input.length - 2) / 2),
            // added: the prover API addresses transactions by index, not hash
            index: block.transactions.findIndex((tx: any) => tx.hash === sampledTransaction.hash),
        },
    };
}

/**
 * Index of the transaction to request a proof for.
 *
 * Index 0 is the worst possible choice: first-in-block skews large (highest gas price), so it
 * lands around the 99th percentile by payload size and routinely blows the gas budget on proofs
 * that are fine for any other transaction in the same block.
 *
 * Falls back to index 0 — the previous behaviour — when no RPC is configured, when the block has
 * no transactions so the prover's existing `EmptyBlockTxProof` skip still handles it, and if the
 * sampled transaction cannot be located in the block. Every other error propagates, including a
 * missing block or an unreachable RPC, since those mean we are not sampling what we think we are.
 */
async function pickTransactionIndex(sourceRpc: JsonRpcProvider | undefined, blockNumber: bigint): Promise<number> {
    if (sourceRpc === undefined) {
        return 0;
    }

    try {
        const sample = await sampleNormalTransaction(sourceRpc, blockNumber);
        return sample.transaction.index < 0 ? 0 : sample.transaction.index;
    } catch (error) {
        if (error instanceof Error && error.message === 'Block contains no transactions') {
            return 0;
        }
        throw error;
    }
}

async function getProofForBlock(apiUrl: string, chainKey: number, blockNumber: bigint, txIndex: number) {
    const url = `${apiUrl}/api/v1/proof/${chainKey}/${blockNumber}/${txIndex}`;
    try {
        // NOTE: throws an exception in case of errors
        return await axios.get(url);
    } catch (error) {
        // The prover returns HTTP 422 with code 'EmptyBlockTxProof' for blocks
        // that contain no transactions; there is no tx proof to verify, so we
        // treat this as a skip rather than a hard failure. Any other error is
        // re-thrown so genuine problems still surface and fail the run.
        if (
            axios.isAxiosError(error) &&
            error.response?.status === 422 &&
            error.response?.data?.code === 'EmptyBlockTxProof'
        ) {
            return null;
        }
        throw error;
    }
}

const fetchCheckpoints = (
    indexerUrl: string,
    chainKey: number,
    startBlock: number,
    lastBlock: number,
    afterCursor: string | null,
) => {
    return graphQLQuery(
        indexerUrl,
        `query {
            checkpoints(
                filter: {
                    chainKey: { equalTo: "${chainKey}" },
                    blockNumber: {
                        greaterThanOrEqualTo: "${startBlock}",
                        lessThanOrEqualTo: "${lastBlock}"
                    },
                },
                orderBy: BLOCK_NUMBER_ASC,
                first: 100,
                after: ${afterCursor},
            ) {
                nodes { blockNumber },
                pageInfo { endCursor, hasNextPage },
            }
        }`,
    );
};

async function main(
    creditcoinWsUrl: string,
    chainKey: number,
    proverBaseUrl: string,
    saveProofsTo: string,
    indexerUrl: string | undefined,
    decoderAddress: string | undefined,
): Promise<void> {
    const creditcoinWs = new WebSocketProvider(creditcoinWsUrl);
    const prover = new blockProver.PrecompileBlockProver(creditcoinWs);
    // NOTE: the SDK does not expose a wrapper for get_latest_checkpoint_height_and_hash yet,
    // so we instantiate the precompile contract directly with the shipped ABI.
    const chainInfoContract = new Contract(chainInfo.CHAIN_INFO_PRECOMPILE_ADDRESS, ChainInfoABI, creditcoinWs);
    const latestCheckpointRaw = await chainInfoContract.get_latest_checkpoint_height_and_hash(chainKey, {
        blockTag: 'finalized',
    });
    const lastCheckpoint = {
        height: Number(latestCheckpointRaw[0]),
        hash: latestCheckpointRaw[1] as string,
        isAttestation: latestCheckpointRaw[2] as boolean,
        exists: latestCheckpointRaw[3] as boolean,
    };
    console.log(
        `**** INFO: ${creditcoinWsUrl}, chainKey=${chainKey}, last checkpoint is for block ${lastCheckpoint.height}`,
    );

    const lastSourceBlock = parseInt(process.env.LAST_SOURCE_BLOCK || lastCheckpoint.height.toString(), 10);
    const goBack = parseInt(process.env.GO_BACK_BLOCKS || '4000', 10); // how many blocks to go back in time
    const startFrom = lastSourceBlock - goBack;
    const stepThrough = parseInt(process.env.STEP_THROUGH_BLOCKS || '5', 10); // how many blocks to step through

    // NOTE: the default is to query random blocks by iterating over them
    const blocksToInspect = Array.from({ length: goBack / stepThrough + 1 }, (value, index) =>
        BigInt(startFrom + index * stepThrough),
    );
    if (indexerUrl === undefined || indexerUrl === '') {
        console.log(
            `**** INFO: will check ${blocksToInspect.length} random blocks: ${startFrom}..${lastSourceBlock}, step ${stepThrough}`,
        );
    } else {
        // NOTE: if indexerUrl is defined will inspect blocks at checkpoint boundaries instead

        // reset which blocks we should inspect
        blocksToInspect.splice(0, blocksToInspect.length);

        let hasNextPage = true;
        let afterCursor = null;
        do {
            // WARNING: this returns max 100 records
            const response = await fetchCheckpoints(indexerUrl, chainKey, startFrom, lastSourceBlock, afterCursor);
            hasNextPage = response.data.checkpoints.pageInfo.hasNextPage;
            afterCursor = `"${response.data.checkpoints.pageInfo.endCursor}"`;

            for (const node of response.data.checkpoints.nodes) {
                // upper boundary of current checkpoint
                blocksToInspect.push(BigInt(node.blockNumber));
                // lower boundary of next checkpoint
                if (BigInt(node.blockNumber) + 1n <= lastCheckpoint.height) {
                    blocksToInspect.push(BigInt(node.blockNumber) + 1n);
                }
            }
        } while (hasNextPage);
        console.log(
            `**** INFO: will check ${blocksToInspect.length} blocks at checkpoint boundaries: ${startFrom}..${lastSourceBlock}`,
        );
    }

    if (blocksToInspect.length === 0) {
        throw new Error('no blocks to inspect. Something is wrong! Investigate!');
    }

    let contract: Contract | undefined;
    if (decoderAddress !== undefined && decoderAddress !== '') {
        contract = new Contract(decoderAddress, EvmV1DecoderABI, creditcoinWs);
    }

    // Read from the environment rather than argv: the URL usually embeds an API key, and argv is
    // echoed by the CI runner. Optional — without it every block falls back to index 0.
    const sourceRpcUrl = process.env.SOURCE_CHAIN_RPC_URL;
    const sourceRpc = sourceRpcUrl ? new JsonRpcProvider(sourceRpcUrl) : undefined;
    console.log(
        sourceRpc === undefined
            ? '**** INFO: no SOURCE_CHAIN_RPC_URL, sampling transaction index 0'
            : '**** INFO: selecting a representative transaction per block via the source chain RPC',
    );

    // NOTE: an ephemeral wallet is used purely as a `from` address for
    // `estimateGas`. No transaction is submitted, so this account never
    // needs to be funded and there is no on-chain side effect.
    const estimateGasSigner = Wallet.createRandom().connect(creditcoinWs);
    const blockProverContractWithSigner = prover.blockProverContract.connect(estimateGasSigner);
    // ABI fragment for the state-changing `verifyAndEmit` precompile entry-point;
    // mirrors the constant used inside @gluwa/usc-sdk's PrecompileBlockProver.
    const verifyAndEmitSingleFragment =
        'verifyAndEmit(uint64,uint64,bytes,(bytes32,(bytes32,bool)[]),(bytes32,bytes32[]))';

    // Read the EVM block gas limit from chain instead of hard-coding it, so this check tracks the
    // runtime's configured `BlockGasLimit` (which changes across releases). Frontier reports that
    // configured limit as the block `gasLimit` over the standard eth RPC, so any finalized block
    // carries the current value. Fetched once since it is constant for a given runtime.
    const latestBlock = await creditcoinWs.getBlock('finalized');
    if (latestBlock === null || latestBlock.gasLimit <= 0n) {
        throw new Error('could not read EVM block gas limit from chain');
    }
    // Three tiers, all derived from the on-chain limit so they track the runtime:
    //   blockGasLimit      hard ceiling; pallet-ethereum rejects a tx above it. estimateGas can
    //                      still report higher, since eth_call is served up to
    //                      `block.gas_limit * --execute-gas-limit-multiplier`.
    //   70%                block budget — see commit log + linked Slack thread.
    //   blockGasLimit / 3  per-transaction tripwire, ahead of per-transaction gas limits landing
    //                      on chain. Equals the 25M it replaces; derived so it cannot go stale.
    const blockGasLimit = latestBlock.gasLimit;
    const totalGasThreshold = (blockGasLimit * 7n) / 10n;
    const singleTxnGasLimit = blockGasLimit / 3n;
    console.log(
        `**** INFO: on-chain EVM block gas limit = ${blockGasLimit} (hard ceiling), 70% threshold = ${totalGasThreshold}, per-txn tripwire = ${singleTxnGasLimit}`,
    );

    const sleepTime = parseInt(process.env.SLEEP_TIME || '500', 10);
    for (const blockNumber of blocksToInspect) {
        await sleep(sleepTime); // rate-limit
        const txIndex = await pickTransactionIndex(sourceRpc, blockNumber);
        console.log(`... get proof for source chain block ${blockNumber}, txIndex ${txIndex}`);
        const response = await getProofForBlock(proverBaseUrl, chainKey, blockNumber, txIndex);
        if (response === null) {
            console.log('    ... skipping verification. Empty block, no tx proof available');
            continue;
        }
        const proofData = response.data as proofProvider.ContinuityResponse;
        if (proofData.txBytes === undefined) {
            console.log('    ... skipping verification. No transactions in block');
            continue;
        }

        await sleep(500); // rate-limit
        console.log(`    ..... trying to verify proof for ${blockNumber} -> ${proofData.txHash}`);
        writeToDisk(saveProofsTo, proofData);

        const verificationResult = await prover.verifySingle(
            proofData.chainKey,
            proofData.headerNumber,
            proofData.txBytes,
            proofData.merkleProof,
            proofData.continuityProof,
        );
        console.log('    ... proof verification:', verificationResult ? 'SUCCESS' : 'FAILED');
        if (!verificationResult) {
            throw new Error('...... proof verification failed');
        }

        // Record the gas it WOULD cost to call verifyAndEmitSingle on this proof.
        // We deliberately call estimateGas rather than submitting the tx so that
        // running prover-check in parallel for the same signer cannot trip on
        // nonce collisions or actually move on-chain state. If estimateGas
        // fails the whole run is allowed to fail so the problem surfaces.
        const estimate = await blockProverContractWithSigner
            .getFunction(verifyAndEmitSingleFragment)
            .estimateGas(
                proofData.chainKey,
                proofData.headerNumber,
                proofData.txBytes,
                proofData.merkleProof,
                proofData.continuityProof,
            );
        const gasForVerification = BigInt(estimate);
        console.log(`    ... gasForVerification=${gasForVerification}`);

        // Check each component against the ceiling as it lands, so an unsubmittable estimate is
        // attributed to verification or decoding rather than only to the combined total.
        if (gasForVerification > blockGasLimit) {
            throw new Error(
                `gasForVerification ${gasForVerification} exceeds the ${blockGasLimit} block gas limit; unsubmittable, failing run`,
            );
        }

        let gasForDecoding = 0n;
        if (contract !== undefined) {
            console.log('    ..... trying to decode proof');
            const decoded = await utils.decoder.decodeEvmV1Transaction(proofData.txBytes, contract, {
                trackGas: true,
            });
            gasForDecoding = decoded.gasUsed ?? 0n;
            console.log(`    ... decoded as type ${decoded.type}, gasForDecoding=${gasForDecoding}`);
        }
        if (gasForDecoding > blockGasLimit) {
            throw new Error(
                `gasForDecoding ${gasForDecoding} exceeds the ${blockGasLimit} block gas limit; unsubmittable, failing run`,
            );
        }

        // 10% safety margin on the raw estimates. Most severe tier first, so an unsubmittable
        // proof does not report as merely over budget.
        const totalGas = ((gasForVerification + gasForDecoding) * 11n) / 10n;
        console.log(`    ... totalGas (with 10% margin)=${totalGas} (threshold=${totalGasThreshold})`);
        if (totalGas > blockGasLimit) {
            throw new Error(
                `totalGas ${totalGas} exceeds the ${blockGasLimit} block gas limit; unsubmittable, failing run`,
            );
        }
        if (totalGas >= totalGasThreshold) {
            throw new Error(
                `totalGas ${totalGas} reaches or exceeds 70% of the ${blockGasLimit} block gas limit (${totalGasThreshold}); failing run`,
            );
        }
        if (totalGas >= singleTxnGasLimit) {
            throw new Error(
                `totalGas ${totalGas} reaches or exceeds the per-transaction tripwire (${singleTxnGasLimit}, a third of the ${blockGasLimit} block gas limit); failing run`,
            );
        }
    }
    console.log('**** INFO: done');
    process.exit(0);
}

if (process.argv.length < 6) {
    console.error(
        'prover-check.js <creditcoinWssUrl> <chainKey> <proverBaseUrl> <saveProofsDir> [<indexerUrl>] [evmV1Decoder address]',
    );
    process.exit(1);
}

const creditcoinWsRpcUrl = process.argv[2];
const sourceChainKey = Number(process.argv[3]);
const proverUrl = process.argv[4];
const saveProofsDir = process.argv[5];
// when defined will query proofs at checkpoint boundaries
// otherwise will query random blocks by iterating over them
const cc3IndexerUrl = process.argv[6];
// when defined will decode proof data against on-chain contract
const evmV1DecoderAddress = process.argv[7];

main(creditcoinWsRpcUrl, sourceChainKey, proverUrl, saveProofsDir, cc3IndexerUrl, evmV1DecoderAddress).catch(
    (reason) => {
        console.error(reason);
        process.exit(1);
    },
);
