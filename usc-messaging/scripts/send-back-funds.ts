import "dotenv/config";
import { ethers } from "ethers";
import { TEST_TOKEN_AMOUNT } from "../src/consts";
import { requireEnv } from "../src/utils";

const DESTINATION_RPC_URL = requireEnv("DESTINATION_CHAIN_RPC_URL");
const DESTINATION_PRIVATE_KEY = requireEnv("DESTINATION_CHAIN_PRIVATE_KEY");
const DESTINATION_CONTRACT_ADDR = requireEnv("DESTINATION_CONTRACT_ADDR");

// ABI for the destination contract sendTokens function
const DESTINATION_ABI = [
  "function sendTokens(uint256 amount) external",
];

async function main() {
  const provider = new ethers.JsonRpcProvider(DESTINATION_RPC_URL);

  const wallet = new ethers.Wallet(
    DESTINATION_PRIVATE_KEY!,
    provider
  );

  const contract = new ethers.Contract(
    DESTINATION_CONTRACT_ADDR!,
    DESTINATION_ABI,
    wallet
  );

  console.log("Sending tokens...");
  console.log(`  from:   ${wallet.address}`);
  console.log(`  amount: ${TEST_TOKEN_AMOUNT.toString()}`);

  const tx = await contract.sendTokens(TEST_TOKEN_AMOUNT);
  console.log(`Tx sent: ${tx.hash}`);

  const receipt = await tx.wait();
  console.log(`Confirmed in block ${receipt.blockNumber}`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});