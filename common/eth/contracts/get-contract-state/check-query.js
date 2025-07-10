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

    if (rawQuery.state === 0) {
      throw new Error("Query state uninitialized. The query does not exist.");
    }

    console.log("✅ Query found!");
    console.log("State (enum):", rawQuery.state);
    console.log("Principal:", rawQuery.principal);
    console.log("Escrowed:", rawQuery.escrowedAmount.toString());
    console.log("Estimated cost:", rawQuery.estimatedCost.toString());
    console.log("Timestamp:", rawQuery.timestamp.toString());

  } catch (err) {
    console.error("❌ Request for query failed");
    console.error(err.message);
  }
}

checkQuery();