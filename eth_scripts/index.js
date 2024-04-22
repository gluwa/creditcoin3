const ethers = require("ethers");
const { hexToU8a, isHex } = require("@polkadot/util");
const { decodeAddress, encodeAddress } = require("@polkadot/keyring");

// Define the contract ABI (Application Binary Interface)
const contractABI = [
  {
    anonymous: false,
    inputs: [
      {
        indexed: false,
        internalType: "bytes32",
        name: "destination",
        type: "bytes32",
      },
      {
        indexed: false,
        internalType: "uint256",
        name: "amount",
        type: "uint256",
      },
    ],
    name: "Transfer",
    type: "event",
  },
  {
    inputs: [
      {
        internalType: "bytes32",
        name: "destination",
        type: "bytes32",
      },
      {
        internalType: "uint256",
        name: "amount",
        type: "uint256",
      },
    ],
    name: "transfer_substrate",
    outputs: [],
    stateMutability: "nonpayable",
    type: "function",
  },
];

const source = "0x5D098dA732b72b6c98a184F7538A2dCB5D13962C";

// Specify the address of the deployed contract
const contractAddress = "0x0000000000000000000000000000000000000fd1"; // Replace this with your contract address

// Specify your private key for authentication
// const privateKey = '8075991ce870b93a8870eca0c0f91913d12f47948ca0fd25b49c6fa7cdbeee8b'; // Replace this with your private key
const privateKey =
  "0x1a3f3571ce332f0ad34fca859b77a4b5e76688d84837c19fdfade7cb700346e3";

// Create an instance of the provider connected to the specified network
provider = new ethers.JsonRpcProvider("http://127.0.0.1:8545/");

// Create a wallet instance using the private key
const wallet = new ethers.Wallet(privateKey, provider);

// Connect to the deployed contract using the contract's ABI and address
const contract = new ethers.Contract(contractAddress, contractABI, wallet);

// Example: Call a function on the contract
async function callContract() {
  const code = await contract.getDeployedCode();
  console.log(`contract code: ${code}`);

  const balance = await provider.getBalance(source);
  console.log(ethers.formatUnits(balance));

  try {
    const toPubkey =
      "0xf615f1b14094e7c32a5d57e60ef5bdd448856acebf2728d00edef61908273270";
    console.log(`bytes ${hexToU8a(toPubkey)}`);

    // Call the transfer function on the contract
    const amount = ethers.parseEther("10.0");

    const gasPrice = (await provider.getFeeData()).gasPrice;
    const result = await contract.transfer_substrate(toPubkey, amount, {
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
