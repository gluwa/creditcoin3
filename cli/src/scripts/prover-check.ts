import { blockProver, chainInfo, proofGenerator } from '@gluwa/usc-sdk';
import { WebSocketProvider } from 'ethers';
import axios from 'axios';

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

async function getProofForBlock(apiUrl: string, chainKey: number, blockNumber: number) {
    const url = `${apiUrl}/api/v1/proof/${chainKey}/${blockNumber}/0`;
    // NOTE: throws an exception in case of errors
    return axios.get(url);
}

async function main(creditcoinWsUrl: string, chainKey: number, proverBaseUrl: string): Promise<void> {
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

    const howMany = Math.ceil(goBack / stepThrough);
    console.log(`**** INFO: will check ${howMany} blocks: ${startFrom}..${lastSourceBlock}, step ${stepThrough}`);

    for (let blockNumber = startFrom; blockNumber < lastSourceBlock; blockNumber += stepThrough) {
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
