import { ethers } from "./node_modules/ethers/dist/ethers.min.js";
import CounterContractArtifact from "../with-hardhat/artifacts/contracts/Counter.sol/Counter.json";

let provider;
let signer = null;
let CounterContract;

const CounterContractAddress = "DEPLOYED_COUNTER_CONTRACT_ADDRESS";
const CounterContractABI = CounterContractArtifact.abi;

if (window.ethereum === null) {
    // If MetaMask is not installed, we use the default provider,
    // which is backed by a variety of third-party services (such
    // as INFURA). They do not have private keys installed,
    // so they only have read-only access
    console.log("MetaMask not installed; using read-only defaults")
    provider = ethers.getDefaultProvider()
} else {
    // Connect to the MetaMask EIP-1193 object. This is a standard
    // protocol that allows Ethers access to make all read-only
    // requests through MetaMask.

    const result = await window.ethereum.request({
      method: "wallet_addEthereumChain",
      params: [{
          chainId: "0x18e8f", // see https://chainlist.org/chain/102031
          rpcUrls: ["https://rpc.cc3-testnet.creditcoin.network"],
          chainName: "Creditcoin Testnet",
          nativeCurrency: {
              name: "tCTC",
              symbol: "tCTC",
              decimals: 18
          },
          blockExplorerUrls: ["https://creditcoin-testnet.blockscout.com/"]
      }]
    });

    // WARNING: this works with ethers.js v6 or later
    provider = new ethers.BrowserProvider(window.ethereum)

    // It also provides an opportunity to request access to write
    // operations, which will be performed by the private key
    // that MetaMask manages for the user.
    signer = await provider.getSigner();
}

// Initialize the CounterContract instance
CounterContract = new ethers.Contract(
  CounterContractAddress,
  CounterContractABI,
  signer
);

// Refresh count function
const refreshCount = async () => {
  const count = await CounterContract.getCount();
  document.getElementById("count").innerHTML = count;
}


// Create contract increment function for button
const increment = async () => {
  const tx = await CounterContract.incrementCounter();
  await tx.wait();
  await refreshCount();
};

// Create contract decrement function for button
const decrement = async () => {
  const tx = await CounterContract.decrementCounter();
  await tx.wait();
  await refreshCount();
};

// Add increment behavior to Increment button
document.getElementById("increment").addEventListener("click", increment);

// Add decrement behavior to Decrement button
document.getElementById("decrement").addEventListener("click", decrement);

// Get the actual count from the chain and update element
await refreshCount();
