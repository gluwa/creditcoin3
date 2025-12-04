const ethers = require("ethers");
const experimental = require('@ethersproject/experimental');

// Define default and devnet provider URLs
const DEFAULT_PROVIDER_URL = "http://127.0.0.1:8141";
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

async function sendTransfers() {
  // Get signer
  const signer = getSigner();
  const publicKey = await signer.getAddress();
  const gasLimit = '0x100000';

  // Handles the nonce for each transaction
  const manager = new experimental.NonceManager(signer);

  for (let i = 0; i < 5; i++) {
    console.log(`**** DEBUG: iteration ${i}`)

    const tx = {
      from: publicKey,
      to: getRandomEthAddress(),
      // a random amount between 0.1 and 1 ETH
      value: ethers.parseEther((Math.random() * (1 - 0.1) + 0.1).toFixed(18)),
      nonce: await signer.provider.getTransactionCount(publicKey, 'latest') + i,
      gasLimit: gasLimit,
      gasPrice: (await signer.provider.getFeeData()).gasPrice,
    };

    // Replace signer with nonce manager
    await manager.sendTransaction(tx).then(receipt => {
      console.log(`Transaction hash: ${receipt.hash}`);
    });
  }
}

// Execute the function
sendTransfers()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error(error);
    process.exit(1);
  });
