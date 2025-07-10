const { ethers } = require("ethers");

const HTTPS_RPC_URL = "http://localhost:9944";
const contractAddress = process.argv[2];
const queryId = process.argv[3];

if (!contractAddress || !queryId) {
  console.error("Usage: node script.js <contractAddress> <queryId>");
  process.exit(1);
}

const abi = require("./prover-abi.json");
const provider = new ethers.JsonRpcProvider(HTTPS_RPC_URL);
const contract = new ethers.Contract(contractAddress, abi, provider);

async function checkQuery() {
  try {
    const rawQuery = await contract.queries(queryId);

    console.log("✅ Query found!");
    console.log("State (enum):", rawQuery.state);
    console.log("Principal:", rawQuery.principal);
    console.log("Escrowed:", rawQuery.escrowedAmount.toString());
    console.log("Estimated cost:", rawQuery.estimatedCost.toString());
    console.log("Timestamp:", rawQuery.timestamp.toString());

    // Optional: dump the whole query object if you're debugging
    // console.dir(rawQuery, { depth: null });
  } catch (err) {
    console.error("❌ Query not found or reverted. Likely does not exist.");
    console.error(err.message);
  }
}

checkQuery();