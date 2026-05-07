import { chainInfo } from '@gluwa/usc-sdk';
import { WebSocketProvider } from 'ethers';
import axios from 'axios';

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

async function getProofForBlock(apiUrl: string, chainKey: number, blockNumber: number) {
    const url = `${apiUrl}/api/v1/proof/${chainKey}/${blockNumber}/0`;
    // NOTE: throws an exception in case of errors
    return axios.get(url);
}

async function main(creditcoinWsUrl: string, chainKey: number, proverBaseUrl: string): Promise<void> {
    const chainInfoPrecompile = new chainInfo.PrecompileChainInfoProvider(new WebSocketProvider(creditcoinWsUrl));
    const lastAttestation = await chainInfoPrecompile.getLatestAttestedHeightAndHash(chainKey);
    console.log(
        `**** INFO: ${creditcoinWsUrl}, chainKey=${chainKey}, last attestation is for block ${lastAttestation.height}`,
    );

    const lastSourceBlock = parseInt(process.env.LAST_SOURCE_BLOCK || lastAttestation.height.toString(), 10);
    const goBack = parseInt(process.env.GO_BACK_BLOCKS || '4000', 10); // how many blocks to go back in time
    const startFrom = lastSourceBlock - goBack;
    const stepThrough = parseInt(process.env.STEP_THROUGH_BLOCKS || '5', 10); // how many blocks to step through

    console.log(`**** INFO: will check ${goBack} blocks: ${startFrom}..${lastSourceBlock}, step ${stepThrough}`);

    for (let blockNumber = startFrom; blockNumber < lastSourceBlock; blockNumber += stepThrough) {
        console.log(`... get proof for source chain block ${blockNumber}`);
        await getProofForBlock(proverBaseUrl, chainKey, blockNumber);

        // Prover talks to Infura so rate limit ourselves
        await sleep(1_000);
    }
    console.log('**** INFO: done');
    process.exit(0);
}

if (process.argv.length < 5) {
    console.error('prover-check.js <creditcoinWssUrl> <chainKey> <proverBaseUrl>');
    process.exit(1);
}

const creditcoinWsRpcUrl = process.argv[2];
const sourceChainKey = Number(process.argv[3]);
const proverUrl = process.argv[4];

main(creditcoinWsRpcUrl, sourceChainKey, proverUrl).catch((reason) => {
    console.error(reason);
    process.exit(1);
});
