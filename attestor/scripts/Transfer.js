const ethers = require("ethers")

function getRandomEthAddress() {
  return ethers.Wallet.createRandom().address
}

function getSigner() {
  const privateKey =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

  // Create an instance of the provider connected to the specified network
  const provider = new ethers.JsonRpcProvider("http://127.0.0.1:8545")

  // Create a wallet instance using the private key
  const wallet = new ethers.Wallet(privateKey, provider)

  return wallet
}

async function main() {
  // Start sending a single transfer
  await sendTransfer()
}

async function sendTransfer() {
  // Get signer
  const signer = getSigner()

  // Generate a random amount between 0.1 and 1 ETH
  const randomAmount = (Math.random() * (1 - 0.1) + 0.1).toFixed(18)
  const value = ethers.parseEther(randomAmount)

  // Generate a random recipient address
  const recipientAddress = getRandomEthAddress()

  // Get the current nonce
  const nonce = await signer.getNonce()

  // Send a single transaction
  const tx = await signer.sendTransaction({
    to: recipientAddress,
    value: value,
    nonce: nonce,
  })

  // Wait for the transfer to be mined
  const receipt = await tx.wait()
  console.log(`Transfer mined in block ${receipt.blockNumber}`)
}

// Execute the function
main()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error(error)
    process.exit(1)
  })
