// Plain-ethers source-stack deploy (no hardhat runtime — it wedges on this RPC).
// Reads compiled artifacts from usc-contracts/artifacts. Run with tsx (Node 22).
import { ethers } from "ethers";
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// usc-contracts checkout: $USC_CONTRACTS_DIR, else the sibling of this repo (…/Projects/usc-contracts).
const UC = process.env.USC_CONTRACTS_DIR ?? fileURLToPath(new URL("../../../usc-contracts", import.meta.url));
const ART = (p: string, n: string) =>
  JSON.parse(readFileSync(`${UC}/artifacts/contracts/${p}/${n}.json`, "utf8"));

const OUT = "/tmp/e2e-deploy.json";
const DEAD = "0x000000000000000000000000000000000000dEaD";
const CHAIN_KEY = 2;
const DEST_CHAIN_ID = 31337;
const QUOTER_EOA = "0xa0Ee7A142d267C1f36714E4a8F75612F20a79720";
const BALTHATHAR = "0x8075991ce870b93a8870eca0c0f91913d12f47948ca0fd25b49c6fa7cdbeee8b";

const provider = new ethers.JsonRpcProvider("http://127.0.0.1:9944", 42, { staticNetwork: true, polling: true });
provider.pollingInterval = 800;
const wallet = new ethers.Wallet(BALTHATHAR, provider);

async function deploy(name: string, art: any, args: any[] = []) {
  const f = new ethers.ContractFactory(art.abi, art.bytecode, wallet);
  const c = await f.deploy(...args);
  const addr = await c.getAddress();
  process.stdout.write(`  ${name} → ${addr}\n`);
  await c.waitForDeployment();
  return c;
}

async function main() {
  const addrs = JSON.parse(readFileSync(OUT, "utf8"));
  if (!addrs.dest?.inbox) throw new Error("need dest.inbox");
  const owner = wallet.address;
  console.log("deployer:", owner, "nonce:", await wallet.getNonce());

  const attest = await deploy("MockERC20", ART("mocks/MockERC20.sol", "MockERC20"), ["Attest", "ATTEST"]);
  const proof = await deploy("USCProofVerifier", ART("write-ability/common/USCProofVerifier.sol", "USCProofVerifier"));
  const decoder = await deploy("EVMDeliveryDecoder", ART("write-ability/common/EVMDeliveryDecoder.sol", "EVMDeliveryDecoder"), [owner]);
  const twap = await deploy("TWAPReader", ART("write-ability/TWAPReader.sol", "TWAPReader"), [owner, owner]);
  // Start the TWAP observation clock NOW with a short window: read() reverts
  // InsufficientPoolHistory until (now - firstObservation) >= window (default 600s). A 60s window
  // + an immediate observation makes the quoter fee-floor usable by the time anything publishes.
  await (await (twap as any).setWindow(60)).wait();
  await (await (twap as any).update(ethers.parseEther("1"))).wait();
  const factory = await deploy("OutboxFactory", ART("write-ability/deployer/OutboxFactory.sol", "OutboxFactory"));
  const quoter = await deploy("USCRelayingQuoter", ART("write-ability/USCRelayingQuoter.sol", "USCRelayingQuoter"), [owner, await twap.getAddress(), owner]);

  const attestAddr = await attest.getAddress();
  const quoterAddr = await quoter.getAddress();
  const RATE = 0n;

  const n = await wallet.getNonce();
  const at = (k: number) => ethers.getCreateAddress({ from: owner, nonce: n + k });
  const [avAddr, fvAddr, rcAddr] = [at(0), at(1), at(2)];
  const obAddr = await (factory as any).computeOutboxAddressFor(owner, CHAIN_KEY, owner, owner, RATE, avAddr, quoterAddr, attestAddr);

  const av = await deploy("AttestorVault", ART("write-ability/AttestorVault.sol", "AttestorVault"),
    [owner, attestAddr, obAddr, rcAddr, owner, DEAD, 0]);
  const fv = await deploy("RelayerFeeVault", ART("write-ability/RelayerFeeVault.sol", "RelayerFeeVault"),
    [owner, attestAddr, rcAddr, await proof.getAddress(), await decoder.getAddress(), quoterAddr]);
  const rc = await deploy("RelayerContract", ART("write-ability/RelayerContract.sol", "RelayerContract"),
    [owner, attestAddr, obAddr, avAddr, fvAddr, quoterAddr]);
  if ((await av.getAddress()) !== avAddr || (await rc.getAddress()) !== rcAddr) throw new Error("precompute mismatch");

  console.log("  deployOutbox via factory…");
  const tx = await (factory as any).deployOutbox(CHAIN_KEY, owner, owner, RATE, avAddr, quoterAddr, attestAddr);
  const rcpt = await tx.wait();
  const created = rcpt.logs.map((l: any) => { try { return (factory as any).interface.parseLog(l); } catch { return null; } })
    .find((e: any) => e?.name === "OutboxCreated");
  const outboxAddr = created.args.outbox as string;
  if (outboxAddr.toLowerCase() !== obAddr.toLowerCase()) throw new Error("outbox CREATE2 mismatch");
  console.log("  Outbox →", outboxAddr);

  const outbox = new ethers.Contract(outboxAddr, ART("write-ability/Outbox.sol", "Outbox").abi, wallet);
  await (await outbox.setTrustedForwarder(rcAddr, true)).wait();

  // Acknowledgment round-trip: deploy the AcknowledgmentValidator for the destination chain key,
  // make it the Outbox's validator (only it may call acknowledgeMessage), then point it at the
  // Outbox. The relayer's ack submitter proves MessageDelivered on the destination and calls
  // submitAcknowledgment here, which flips messages[id].acknowledged = true on the Outbox.
  const ackValidator = await deploy("AcknowledgmentValidator",
    ART("write-ability/AcknowledgementValidator.sol", "AcknowledgmentValidator"), [CHAIN_KEY, owner]);
  const ackAddr = await ackValidator.getAddress();
  await (await outbox.setValidator(ackAddr)).wait();
  await (await (ackValidator as any).setOutbox(outboxAddr)).wait();
  console.log("  ack: validator", ackAddr, "→ Outbox");
  await (await (quoter as any).addQuoter(QUOTER_EOA)).wait();
  await (await (quoter as any).setCoreFee(CHAIN_KEY, ethers.parseEther("1"))).wait();
  await (await (decoder as any).setTrustedInbox(DEST_CHAIN_ID, addrs.dest.inbox)).wait();
  await (await (fv as any).setDestinationEvmChainId(CHAIN_KEY, DEST_CHAIN_ID)).wait();
  console.log("  wired: forwarder, quoter EOA, coreFee, trustedInbox, destEvmChainId");

  // seed prices (deployer == oracleService)
  await (await (twap as any).update(ethers.parseEther("1"))).wait();
  await (await (quoter as any).setPricingMode(0, 10_000_000_000n)).wait();
  await (await (quoter as any).priceUpdate(10_000_000_000n, CHAIN_KEY, [0n, 1n, 10_000_000_000n, 10_000_000_000n, 0])).wait();
  console.log("  seeded quoter prices");

  addrs.source = {
    chainId: 42, rpc: "http://127.0.0.1:9944", chainKey: CHAIN_KEY,
    attest: attestAddr, proofVerifier: await proof.getAddress(), deliveryDecoder: await decoder.getAddress(),
    twapReader: await twap.getAddress(), quoter: quoterAddr, factory: await factory.getAddress(),
    attestorVault: avAddr, outbox: outboxAddr, relayerFeeVault: fvAddr, relayerContract: rcAddr,
    ackValidator: ackAddr,
    quoterEOA: QUOTER_EOA, coreFee: ethers.parseEther("1").toString(),
  };
  writeFileSync(OUT, JSON.stringify(addrs, null, 2));
  console.log("✅ source stack deployed + seeded. Outbox:", outboxAddr, "RelayerContract:", rcAddr);
  process.exit(0);
}
main().catch((e) => { console.error("FAILED:", e.shortMessage ?? e.message); process.exit(1); });
