const { ethers } = require("ethers");

// Replace with your actual values
// const HTTPS_RPC_URL = "https://rpc.ccnext-devnet.creditcoin.network";
const HTTPS_RPC_URL = "http://localhost:9944";

// ABI with only get_result_segments
const abi = require("./prover-abi.json");

// Entry point
async function main() {
  const contractAddress = process.argv[2];
  const queryId = process.argv[3];

  if (!contractAddress || !/^0x[0-9a-fA-F]{40}$/.test(contractAddress)) {
    console.error("Usage: node script.js <contractAddress (40-byte hex string)> <queryId (32-byte hex string)>");
    process.exit(1);
  }

  if (!queryId || !/^0x[0-9a-fA-F]{64}$/.test(queryId)) {
    console.error("Usage: node script.js <contractAddress (40-byte hex string)> <queryId (32-byte hex string)>");
    process.exit(1);
  }

  const provider = new ethers.JsonRpcProvider(HTTPS_RPC_URL);
  const contract = new ethers.Contract(contractAddress, abi, provider);

  try {
    const result = await contract.getQueryDetails(queryId);

    console.log("Raw result:", result);

    console.log("Result Segments:");
    result.resultSegments.forEach((segment, i) => {
      console.log(`Segment ${i}:`);
      console.log(`  Offset: ${segment.offset}`);
      console.log(`  ABI Bytes (hex): ${segment.abiBytes}`);
    });
  } catch (err) {
    console.error("Error calling getQueryDetails():", err);
  }
}

main();
