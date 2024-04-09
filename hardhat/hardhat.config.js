require("@nomicfoundation/hardhat-toolbox");

/** @type import('hardhat/config').HardhatUserConfig */
module.exports = {
  solidity: "0.8.24",
  networks: {
    hardhat: {
      gas: "auto",
      mining: {
        auto: false, // Enable auto mining
        interval: 8000 // Mine a new block every 5 seconds
      }
    }
  }
};
