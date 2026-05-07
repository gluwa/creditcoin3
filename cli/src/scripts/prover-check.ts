import { chainInfo } from '@gluwa/usc-sdk';
import { WebSocketProvider } from 'ethers';
import axios from 'axios';

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

    const startFrom = lastAttestation.height;
    const goBack = 4000; // how many blocks to go back in time
    const stepThrough = 5; // how many blocks to step through

    for (let blockNumber = startFrom - goBack; blockNumber < lastAttestation.height; blockNumber += stepThrough) {
        console.log(`... get proof for source chain block ${blockNumber}`);
        await getProofForBlock(proverBaseUrl, chainKey, blockNumber);
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
