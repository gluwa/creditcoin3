const ethers = require("ethers");
const { mnemonicGenerate } = require("@polkadot/util-crypto");
const { Keyring } = require("@polkadot/keyring");
const contractABI = require("./abi.json");

// Specify the address of the deployed contract
const PRECOMPILE_CONTRACT_ADDRESS =
  "0x0000000000000000000000000000000000000fd1";

// Specify your private key for authentication (Alith in this case)
// Or import one...
const ALITH =
  "5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133";

// Create an instance of the provider connected to the specified network
const provider = new ethers.JsonRpcProvider("http://127.0.0.1:8545/");

// Create a wallet instance using the private key
const wallet = new ethers.Wallet(ALITH, provider);

// Connect to the deployed contract using the contract's ABI and address
const contract = new ethers.Contract(
  PRECOMPILE_CONTRACT_ADDRESS,
  contractABI,
  wallet
);

// Example: Call a function on the contract
async function callContract() {
  // Check balance
  const balance = await provider.getBalance(wallet.address);

  if (balance == 0) {
    throw Error("No funds to call transaction");
  }

  try {
    // Generate a keypair
    // Or import one...
    let target = new Keyring();
    let pair = target.addFromMnemonic(mnemonicGenerate());

    // Call the transfer function on the contract
    const amount = ethers.parseEther("10.0");

    console.log(`Sending ${amount} to ${pair.address}`);

    const gasPrice = (await provider.getFeeData()).gasPrice;
    const result = await contract.transfer_substrate(pair.addressRaw, amount, {
      gasPrice: gasPrice,
    });

    console.log("Transaction result:", result);
    const receipt = await result.wait();
    console.log(`Transaction receipt: ${receipt}`);
  } catch (error) {
    console.error("Error:", error);
  }
}

// Call the function
callContract();
