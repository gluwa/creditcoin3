import { HardhatUserConfig } from 'hardhat/config';
import '@nomicfoundation/hardhat-toolbox';
import 'solidity-coverage';

const config: HardhatUserConfig = {
    solidity: '0.8.24',
    networks: {
        creditcoinLocal: {
            url: 'http://127.0.0.1:9944',
            // EVM private keys for development accounts are documented at
            // https://docs.moonbeam.network/builders/get-started/networks/moonbeam-dev/#pre-funded-development-accounts
            // this list has a direct effect on what ethers.getSigners() returns
            accounts: [
                // Balthathar
                '0x8075991ce870b93a8870eca0c0f91913d12f47948ca0fd25b49c6fa7cdbeee8b',
                // Charleth
                '0x0b6e18cafb6ed99687ec547bd28139cafdd2bffe70e6b688025de6b445aa5c5b',
                // Dorothy
                '0x39539ab1876910bbf3a223d84a29e28f1cb4e2e456503e7e91ed39b2e7223d68',
            ],
        },
        hardhatLocal: {
            url: 'http://127.0.0.1:8545',
            // EVM private keys are printed when starting npx hardhat node
            // This is Account #0
            accounts: ['0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80'],
        },
    },
};

export default config;
