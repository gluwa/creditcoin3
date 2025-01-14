const ethers = require("ethers");

// Define default and devnet provider URLs
const DEFAULT_PROVIDER_URL = "http://127.0.0.1:8545";
const DEVNET_PROVIDER_URL = "https://anvil.ccnext-devnet.creditcoin.network";

function getRandomEthAddress() {
  return ethers.Wallet.createRandom().address;
}

function getSigner() {
  // Anvil Account #0
  const privateKey =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

  // Determine provider URL based on CLI argument
  const providerUrl = process.argv.includes("--devnet")
    ? DEVNET_PROVIDER_URL
    : DEFAULT_PROVIDER_URL;

  // Create an instance of the provider connected to the specified network
  const provider = new ethers.JsonRpcProvider(providerUrl);

  // Create a wallet instance using the private key
  const wallet = new ethers.Wallet(privateKey, provider);

  return wallet;
}

async function sendSingleTransfer() {
  // Get signer
  const signer = getSigner();

  // Generate a random amount between 0.1 and 1 ETH
  const randomAmount = (Math.random() * (1 - 0.1) + 0.1).toFixed(18);
  const value = ethers.parseEther(randomAmount);

  // Generate a random recipient address
  const recipientAddress = getRandomEthAddress();

  // Send the transaction
  const tx = await signer.sendTransaction({
    to: recipientAddress,
    value: value,
  });

  // Wait for the transaction to be mined
  const receipt = await tx.wait();

  // Log the block number and transaction hash
  console.log(`Transaction mined in block ${receipt.blockNumber}`);
  console.log(`Transaction hash: ${receipt.hash}`);
}

// Execute the function
sendSingleTransfer()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error(error);
    process.exit(1);
  });
