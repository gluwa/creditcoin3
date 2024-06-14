const ethers = require("ethers")

function getRandomEthAddress() {
  return ethers.Wallet.createRandom().address
}

function getSigner() {
  const privateKey =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

  // Create an instance of the provider connected to the specified network
  provider = new ethers.JsonRpcProvider("http://127.0.0.1:8545")

  // Create a wallet instance using the private key
  const wallet = new ethers.Wallet(privateKey, provider)

  return wallet
}

async function main() {
  // Start sending transfers continuously
  await sendTransfers()
}

async function sendTransfers() {
  // Get signer
  const signer = getSigner()

  while (true) {
    // Generate a random number of transfers (1-5)
    const numberOfTransfers = Math.floor(Math.random() * 5) + 1

    console.log(`Sending ${numberOfTransfers} transfers...`)

    let promises = []
    // Perform the transfers
    let nonce = await signer.getNonce()

    for (let i = 0; i < numberOfTransfers; i++) {
      // Generate a random amount between 0.1 and 1 ETH
      const randomAmount = (Math.random() * (1 - 0.1) + 0.1).toFixed(18)
      const value = ethers.parseEther(randomAmount)

      // Generate a random recipient address
      const recipientAddress = getRandomEthAddress()

      const tx = await signer.sendTransaction({
        to: recipientAddress,
        value: value,
        nonce: nonce,
      })

      nonce++
      promises.push(tx.wait())
    }

    // Wait for all the transfers to be mined
    let res = await Promise.all(promises)
    res.forEach((tx) => {
      console.log(`Transfer mined in block ${tx.blockNumber}`)
    })

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
