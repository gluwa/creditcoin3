const { ethers } = require("hardhat")

function getRandomEthAddress() {
  return ethers.Wallet.createRandom().address
}

async function main() {
  // Start sending transfers continuously
  await sendTransfers()
}

async function sendTransfers() {
  // Get signer
  const [signer] = await ethers.getSigners()

  while (true) {
    // Generate a random number of transfers (1-5)
    const numberOfTransfers = Math.floor(Math.random() * 5) + 1

    console.log(`Sending ${numberOfTransfers} transfers...`)

    // Perform the transfers
    for (let i = 0; i < numberOfTransfers; i++) {
      // Generate a random amount between 0.1 and 1 ETH
      const randomAmount = (Math.random() * (1 - 0.1) + 0.1).toFixed(18)
      const value = ethers.parseEther(randomAmount)

      // Generate a random recipient address
      const recipientAddress = getRandomEthAddress()

      const tx = await signer.sendTransaction({
        to: recipientAddress,
        value: value,
      })

      console.log(
        `Transfer ${
          i + 1
        } successful! ${randomAmount} ETH has been sent to ${recipientAddress} in tx ${
          tx.hash
        }, block number ${tx.blockNumber}`
      )
    }

    // Schedule the next batch of transfers in 2-4 seconds
    const delay = Math.floor(Math.random() * 2000) + 2000
    await new Promise((resolve) => setTimeout(resolve, delay))
  }
}

// Execute the function
main()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error(error)
    process.exit(1)
  })
