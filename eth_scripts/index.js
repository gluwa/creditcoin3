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
const contractAddress = "0x0000000000000000000000000000000000000801"; // Replace this with your contract address

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
    // Specify the recipient address as a 32-byte hexadecimal string
    //  const toAddress = "5Hb5bu243bFZcs7E8Qjv1pRVaTyKw1nL6Q1KvVMRSApaFPES"; // Example address
    // const toAddress = "5EZzUWruaxVgcV38FWuBZ59SFiP3mgsyuQvoDgk83SBAxbSe";

    // Alice
    const toAddress = "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY";
    const toPubkey =
      "0xf457bb0e786485e6ac1e63fc0fa511a0f72d319e3dc57a65da699d87cfe3284e";
    console.log(`bytes ${hexToU8a(toPubkey)}`);

    // const p = ethers.encodeBytes32String(toAddress);
    // console.log(p);
    // let hash = ethers.sha256(destination);
    // console.log(hash);

    // Call the transfer function on the contract
    const amount = ethers.parseEther("10.0");

    let options = { value: amount };
    const result = await contract.transfer_substrate(toPubkey, amount, options);
    console.log("Transaction result:", result);
    const r = await result.wait();
    console.log(r);
  } catch (error) {
    console.error("Error:", error);
  }
}

// Call the function
callContract();
