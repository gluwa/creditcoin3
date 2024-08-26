import { HardhatUserConfig, vars } from "hardhat/config";
import "@nomicfoundation/hardhat-toolbox";

// Replace this with your Creditcoin3 Testnet account private key
// open MetaMask and go to Account Details > Export Private Key
// Beware: NEVER put real CTC into testing accounts
const CC3TEST_PRIVATE_KEY = vars.get("CC3TEST_PRIVATE_KEY");

const config: HardhatUserConfig = {
  solidity: "0.8.24",
  networks: {
    creditcoin_devnet: {
      url: "https://rpc.cc3-devnet.creditcoin.network",
      accounts: [CC3TEST_PRIVATE_KEY]
    },
    creditcoin_testnet: {
      url: "https://rpc.cc3-testnet.creditcoin.network",
      accounts: [CC3TEST_PRIVATE_KEY]
    },
    creditcoin_mainnet: {
      url: "https://rpc.cc3-mainnet.creditcoin.network",
      accounts: [CC3TEST_PRIVATE_KEY]
    }
  }
};

export default config;
