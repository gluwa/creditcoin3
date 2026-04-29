import { HardhatUserConfig, task, vars } from 'hardhat/config';
import '@nomicfoundation/hardhat-toolbox';

// Replace this with your Creditcoin3 Testnet account private key
// open MetaMask and go to Account Details > Export Private Key
// Beware: NEVER put real CTC into testing accounts
const CC3TEST_PRIVATE_KEY = vars.get('CC3TEST_PRIVATE_KEY');

const config: HardhatUserConfig = {
    solidity: '0.8.24',
    networks: {
        creditcoinDevnet: {
            url: 'https://rpc.usc-devnet.creditcoin.network',
            accounts: [CC3TEST_PRIVATE_KEY],
        },
        creditcoinTestnet: {
            url: 'https://rpc.usc-testnet2.creditcoin.network',
            accounts: [CC3TEST_PRIVATE_KEY],
        },
        creditcoinMainnet: {
            url: 'https://rpc.usc-mainnet.creditcoin.network',
            accounts: [CC3TEST_PRIVATE_KEY],
        },
    },
};

export default config;

// eslint-disable-next-line @typescript-eslint/require-await
task('print-network', 'Prints the value of --network', async (taskArgs, hre) => {
    console.log((hre.network.config as any).url);
});
