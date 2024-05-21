import { HardhatUserConfig } from "hardhat/config";
import "@nomicfoundation/hardhat-toolbox";

const config: HardhatUserConfig = {
  solidity: "0.8.24",
  networks: {
    creditcoin_local: {
      url: "http://127.0.0.1:9944",
      // EVM private keys for development accounts are documented at
      // https://docs.moonbeam.network/builders/get-started/networks/moonbeam-dev/#pre-funded-development-accounts
      // This is the account Balthathar !
      accounts: ["0x8075991ce870b93a8870eca0c0f91913d12f47948ca0fd25b49c6fa7cdbeee8b"]
    }
  }
};

export default config;
