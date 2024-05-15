require("@nomicfoundation/hardhat-toolbox")

/** @type import('hardhat/config').HardhatUserConfig */
module.exports = {
  solidity: "0.8.24",
  networks: {
    hardhat: {
      gas: "auto",
      mining: {
        auto: true, // Enable auto mining
        // interval: 8000 // Mine a new block every 5 seconds
      },
    },
    hardhat2: {
      url: "http://127.0.0.1:8546",
      gas: "auto",
      mining: {
        auto: true, // Enable auto mining
        // interval: 8000 // Mine a new block every 5 seconds
      },
    },
  },
}
