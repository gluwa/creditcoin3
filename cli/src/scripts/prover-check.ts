import { blockProver, chainInfo, proofGenerator, utils } from '@gluwa/usc-sdk';
import EvmV1DecoderABI from '@gluwa/usc-sdk/dist/utils/evmV1DecoderAbi.json';
import { Contract, WebSocketProvider } from 'ethers';
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

async function getProofForBlock(apiUrl: string, chainKey: number, blockNumber: bigint) {
    const url = `${apiUrl}/api/v1/proof/${chainKey}/${blockNumber}/0`;
    // NOTE: throws an exception in case of errors
    return axios.get(url);
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
    indexerUrl: string | undefined,
    decoderAddress: string | undefined,
): Promise<void> {
    const creditcoinWs = new WebSocketProvider(creditcoinWsUrl);
    const prover = new blockProver.PrecompileBlockProver(creditcoinWs);
    const chainInfoPrecompile = new chainInfo.PrecompileChainInfoProvider(creditcoinWs);
    const lastAttestation = await chainInfoPrecompile.getLatestAttestedHeightAndHash(chainKey);
    console.log(
        `**** INFO: ${creditcoinWsUrl}, chainKey=${chainKey}, last attestation is for block ${lastAttestation.height}`,
    );

    const lastSourceBlock = parseInt(process.env.LAST_SOURCE_BLOCK || lastAttestation.height.toString(), 10);
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
                if (BigInt(node.blockNumber) + 1n <= lastAttestation.height) {
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

    for (const blockNumber of blocksToInspect) {
        console.log(`... get proof for source chain block ${blockNumber}`);
        await sleep(500); // rate-limit
        const response = await getProofForBlock(proverBaseUrl, chainKey, blockNumber);
        const proofData = response.data as proofGenerator.ContinuityResponse;
        if (proofData.txBytes === undefined) {
            console.log('    ... skipping verification. No transactions in block');
            continue;
        }

        await sleep(500); // rate-limit
        console.log(`    ..... trying to verify proof for ${blockNumber} -> ${proofData.txHash}`);
        console.log(JSON.stringify(proofData));

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

        if (contract !== undefined) {
            console.log('    ..... trying to decode proof');
            const decoded = await utils.decoder.decodeEvmV1Transaction(proofData.txBytes, contract);
            console.log(`    ... decoded as type ${decoded.type}`);
        }
    }
    console.log('**** INFO: done');
    process.exit(0);
}

if (process.argv.length < 5) {
    console.error(
        'prover-check.js <creditcoinWssUrl> <chainKey> <proverBaseUrl> [<indexerUrl>] [evmV1Decoder address]',
    );
    process.exit(1);
}

const creditcoinWsRpcUrl = process.argv[2];
const sourceChainKey = Number(process.argv[3]);
const proverUrl = process.argv[4];
// when defined will query proofs at checkpoint boundaries
// otherwise will query random blocks by iterating over them
const cc3IndexerUrl = process.argv[5];
// when defined will decode proof data against on-chain contract
const evmV1DecoderAddress = process.argv[6];

main(creditcoinWsRpcUrl, sourceChainKey, proverUrl, cc3IndexerUrl, evmV1DecoderAddress).catch((reason) => {
    console.error(reason);
    process.exit(1);
});
