import { creditcoinApi } from '../lib/api';

async function checkEthBlockNumber(wsUrl: string): Promise<void> {
    const { api } = await creditcoinApi(wsUrl);

    try {
        const ethBlockNumber = await api.rpc.eth.blockNumber();

        console.log(`DEBUG: eth.blockNumber=${ethBlockNumber.toString()}`);

        // means the ETH compatibility layer isn't ready yet
        if (ethBlockNumber.toNumber() < 3) {
            throw new Error('eth.blockNumber() is zero');
        }
    } finally {
        await api.disconnect();
    }
}

if (process.argv.length < 3) {
    console.error('USAGE: eth-check.ts <wsUrl>');
    process.exit(1);
}

const inputWsUrl = process.argv[2];

checkEthBlockNumber(inputWsUrl).catch((reason) => {
    console.error(`ERROR: ${reason}`);
    process.exit(1);
});
