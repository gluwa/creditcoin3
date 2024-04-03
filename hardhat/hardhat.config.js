require("@nomicfoundation/hardhat-toolbox");

/** @type import('hardhat/config').HardhatUserConfig */
module.exports = {
  solidity: "0.8.24",
  networks: {
    hardhat: {
      mining: {
        auto: true, // Enable auto mining
        interval: 5000 // Mine a new block every 5 seconds
      }
    }
  }
};
