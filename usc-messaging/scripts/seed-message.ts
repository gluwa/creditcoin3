#!/usr/bin/env npx tsx
/**
 * Creates a sample messages.json for PoC testing.
 * Run after deploy. Uses deployments.json for TestDestination address.
 */

import { readFile, writeFile } from "fs/promises";
import { existsSync } from "fs";
import { join, dirname } from "path";
import { fileURLToPath } from "url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

async function main() {
  const deployPath = join(ROOT, "deployments.json");
  if (!existsSync(deployPath)) {
    console.error("Run 'npm run deploy' first (with Anvil running)");
    process.exit(1);
  }
  const d = JSON.parse(await readFile(deployPath, "utf-8"));
  const destination = d.destination;
  if (!destination) {
    console.error("deployments.json missing 'destination'");
    process.exit(1);
  }

  const messages = [
    {
      messageId: "0x0000000000000000000000000000000000000000000000000000000000000001",
      emitterAddress: "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
      destinationContract: destination,
      payloadData: "0x48656c6c6f2055534321", // "Hello USC!"
      votes: "0x",
    },
  ];

  const outPath = join(ROOT, "messages.json");
  await writeFile(outPath, JSON.stringify({ messages }, null, 2));
  console.log(`Wrote ${outPath} with 1 sample message -> ${destination}`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
