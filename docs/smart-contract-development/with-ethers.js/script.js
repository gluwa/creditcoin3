import { ethers } from "./node_modules/ethers/dist/ethers.min.js";
import CounterContractArtifact from "../with-hardhat/artifacts/contracts/Counter.sol/Counter.json";

let provider;
let signer = null;

// WARNING: this address depends on which chain the contract was deployed to
// see blockchainTarget selection further below!
const CounterContractAddress = "CHANGE_TO_REAL_DEPLOYMENT_ADDRESS";

// define possible deployment targets here
const blockchainTarget = {
  creditcoin_local: {
    chainId: "0x2a",
    rpcUrls: ["http://127.0.0.1:9944"],
    // WARNING: when testing against a local creditcoin3-node execute it with
    // --dev or --rps-cors all, otherwise some browsers prevent the connection
    // and the eth_chainId method fails => MetaMask fails to add the new chain!
    chainName: "Creditcoin Local",
    nativeCurrency: {
      name: "lCTC",
      symbol: "lCTC",
      decimals: 18
    },
    blockExplorerUrls: null
  },
  hardhat_local: {
    chainId: "0x7a69", // see https://chainlist.org/chain/102031
    rpcUrls: ["http://127.0.0.1:8545"],
    chainName: "Hardhat Local",
    nativeCurrency: {
      name: "tETH",
      symbol: "tETH",
      decimals: 18
    },
    blockExplorerUrls: null
  },
  creditcoin_testnet: {
    chainId: "0x18e8f", // see https://chainlist.org/chain/102031
    rpcUrls: ["https://rpc.cc3-testnet.creditcoin.network"],
    chainName: "Creditcoin Testnet",
    nativeCurrency: {
      name: "tCTC",
      symbol: "tCTC",
      decimals: 18
    },
    blockExplorerUrls: ["https://creditcoin-testnet.blockscout.com/"]
  }
};

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
      params: [blockchainTarget.creditcoin_testnet]
    });

    // WARNING: this works with ethers.js v6 or later
    provider = new ethers.BrowserProvider(window.ethereum)

    // It also provides an opportunity to request access to write
    // operations, which will be performed by the private key
    // that MetaMask manages for the user.
    signer = await provider.getSigner();
}

// Initialize the CounterContract instance
const CounterContract = new ethers.Contract(
  CounterContractAddress,
  CounterContractArtifact.abi,
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
