// usc-dev source-stack deploy (Creditcoin devnet EVM, chain id 42) against CURRENT usc-contracts
// main (post-#23 redesign: RelayerContract owns verifier/decoder, RelayerFeeVault is a dumb vault,
// AcknowledgmentValidator takes proofVerifier+attestToken, AttestorVault needs an AttestorRegistry).
// Env: CC_RPC (default devnet), DEPLOYER_KEY, optional QUOTER_EOA (defaults to deployer).
import { ethers } from "ethers";
import { readFileSync, writeFileSync } from "node:fs";

function ascContractsDir(): string {
  // ASC_CONTRACTS_DIR since the repo was renamed usc-contracts -> asc-contracts; the old name is
  // still accepted so existing setups keep working. Deliberately no default: this used to fall
  // back to one developer's home directory, so anyone else got a confusing "cannot read artifact"
  // three calls later instead of being told what to set.
  const dir = process.env.ASC_CONTRACTS_DIR ?? process.env.USC_CONTRACTS_DIR;
  if (!dir) {
    throw new Error(
      "set ASC_CONTRACTS_DIR to a compiled asc-contracts checkout (run `npx hardhat compile` there first)",
    );
  }
  return dir;
}

const UC = ascContractsDir();
const ART = (p: string, n: string) =>
  JSON.parse(readFileSync(`${UC}/artifacts/contracts/${p}/${n}.json`, "utf8"));

const OUT = process.env.DEPLOY_OUT ?? "/tmp/usc-dev-deploy.json";
const DEAD = "0x000000000000000000000000000000000000dEaD";
const CHAIN_KEY = 8;
const DEST_CHAIN_ID = 11155111; // Sepolia
const CC_CHAIN_ID = 42;

const rpc = process.env.CC_RPC ?? "https://rpc.usc-devnet.creditcoin.network";
const key = process.env.DEPLOYER_KEY!;
if (!key) throw new Error("need DEPLOYER_KEY");

const provider = new ethers.JsonRpcProvider(rpc, CC_CHAIN_ID, { staticNetwork: true, polling: true });
provider.pollingInterval = 1000;
const wallet = new ethers.Wallet(key, provider);
const QUOTER_EOA = process.env.QUOTER_EOA ?? wallet.address;

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
  if (!addrs.dest?.inbox) throw new Error("need dest.inbox — run deploy-dest-devnet first");
  const owner = wallet.address;
  console.log("deployer:", owner, "balance:", ethers.formatEther(await provider.getBalance(owner)), "CTC, nonce:", await wallet.getNonce());

  const attest = await deploy("MockERC20", ART("mocks/MockERC20.sol", "MockERC20"), ["Attest", "ATTEST"]);
  const proof = await deploy("USCProofVerifier", ART("write-ability/common/USCProofVerifier.sol", "USCProofVerifier"));
  const decoder = await deploy("EVMDeliveryDecoder", ART("write-ability/common/EVMDeliveryDecoder.sol", "EVMDeliveryDecoder"), [owner]);
  const twap = await deploy("TWAPReader", ART("write-ability/TWAPReader.sol", "TWAPReader"), [owner, owner]);
  await (await (twap as any).setWindow(60)).wait();
  await (await (twap as any).update(ethers.parseEther("1"))).wait();
  // Registry starts empty on devnet; owner can updateAttestorSet later if the vault flow needs it.
  const registry = await deploy("AttestorRegistry", ART("write-ability/AttestorRegistry.sol", "AttestorRegistry"), [owner, []]);
  const factory = await deploy("OutboxFactory", ART("write-ability/deployer/OutboxFactory.sol", "OutboxFactory"));
  const quoter = await deploy("USCRelayingQuoter", ART("write-ability/USCRelayingQuoter.sol", "USCRelayingQuoter"), [owner, await twap.getAddress(), owner]);

  // Outbox.coreFee() reads through IFeeRegistry, NOT the quoter. This slot used to be handed the
  // quoter, which is a different interface: coreFee() then reverted, and fix-fee-registry.mts
  // existed to swap the real registry in after every deploy. FeeRegistry forwards to this chain's
  // own chain-info precompile (ICoreFeeProvider.get_core_fee(uint32), AddressU64<4051> per
  // runtime/src/precompiles.rs), so fee policy stays in the runtime rather than in contract state.
  const CORE_FEE_PRECOMPILE = "0x0000000000000000000000000000000000000FD3";
  const feeRegistry = await deploy("FeeRegistry", ART("write-ability/FeeRegistry.sol", "FeeRegistry"), [CORE_FEE_PRECOMPILE]);

  const attestAddr = await attest.getAddress();
  const quoterAddr = await quoter.getAddress();
  const proofAddr = await proof.getAddress();
  const decoderAddr = await decoder.getAddress();
  const feeRegistryAddr = await feeRegistry.getAddress();
  const RATE = 0n;

  // av (nonce n), fv (n+1), rc (n+2); Outbox via CREATE2 — resolves the ctor circularity.
  const n = await wallet.getNonce();
  const at = (k: number) => ethers.getCreateAddress({ from: owner, nonce: n + k });
  const [avAddr, fvAddr, rcAddr] = [at(0), at(1), at(2)];
  const obAddr = await (factory as any).computeOutboxAddressFor(owner, CHAIN_KEY, owner, owner, RATE, avAddr, feeRegistryAddr, attestAddr);

  const av = await deploy("AttestorVault", ART("write-ability/AttestorVault.sol", "AttestorVault"),
    [owner, attestAddr, obAddr, rcAddr, owner, await registry.getAddress(), DEAD, 0]);
  const fv = await deploy("RelayerFeeVault", ART("write-ability/RelayerFeeVault.sol", "RelayerFeeVault"),
    [attestAddr, rcAddr]);
  const rc = await deploy("RelayerContract", ART("write-ability/RelayerContract.sol", "RelayerContract"),
    [owner, attestAddr, obAddr, quoterAddr, proofAddr, decoderAddr]);
  if ((await av.getAddress()) !== avAddr || (await rc.getAddress()) !== rcAddr) throw new Error("precompute mismatch");

  console.log("  deployOutbox via factory…");
  const tx = await (factory as any).deployOutbox(CHAIN_KEY, owner, owner, RATE, avAddr, feeRegistryAddr, attestAddr);
  const rcpt = await tx.wait();
  const created = rcpt.logs.map((l: any) => { try { return (factory as any).interface.parseLog(l); } catch { return null; } })
    .find((e: any) => e?.name === "OutboxCreated");
  const outboxAddr = created.args.outbox as string;
  if (outboxAddr.toLowerCase() !== obAddr.toLowerCase()) throw new Error("outbox CREATE2 mismatch");
  console.log("  Outbox →", outboxAddr);

  const outbox = new ethers.Contract(outboxAddr, ART("write-ability/Outbox.sol", "Outbox").abi, wallet);
  await (await outbox.setTrustedForwarder(rcAddr, true)).wait();

  const ackValidator = await deploy("AcknowledgmentValidator",
    ART("write-ability/AcknowledgementValidator.sol", "AcknowledgmentValidator"),
    [CHAIN_KEY, owner, proofAddr, attestAddr]);
  const ackAddr = await ackValidator.getAddress();
  await (await outbox.setValidator(ackAddr)).wait();
  await (await (ackValidator as any).setOutbox(outboxAddr)).wait();
  console.log("  ack: validator", ackAddr, "→ Outbox");

  await (await (rc as any).setRelayerFeeVault(fvAddr)).wait();
  await (await (rc as any).setDestinationEvmChainId(CHAIN_KEY, DEST_CHAIN_ID)).wait();
  await (await (quoter as any).addQuoter(QUOTER_EOA)).wait();
  await (await (quoter as any).setCoreFee(CHAIN_KEY, ethers.parseEther("1"))).wait();
  await (await (decoder as any).setTrustedInbox(DEST_CHAIN_ID, addrs.dest.inbox)).wait();
  console.log("  wired: forwarder, feeVault, destEvmChainId, quoter EOA, coreFee, trustedInbox");

  await (await (twap as any).update(ethers.parseEther("1"))).wait();
  await (await (quoter as any).setPricingMode(0, 10_000_000_000n)).wait();
  await (await (quoter as any).priceUpdate(10_000_000_000n, CHAIN_KEY, [0n, 1n, 10_000_000_000n, 10_000_000_000n, 0])).wait();
  console.log("  seeded quoter prices");

  addrs.source = {
    chainId: CC_CHAIN_ID, rpc, chainKey: CHAIN_KEY,
    attest: attestAddr, proofVerifier: proofAddr, deliveryDecoder: decoderAddr,
    twapReader: await twap.getAddress(), attestorRegistry: await registry.getAddress(),
    quoter: quoterAddr, feeRegistry: feeRegistryAddr, factory: await factory.getAddress(),
    attestorVault: avAddr, outbox: outboxAddr, relayerFeeVault: fvAddr, relayerContract: rcAddr,
    ackValidator: ackAddr, quoterEOA: QUOTER_EOA, coreFee: ethers.parseEther("1").toString(),
  };
  writeFileSync(OUT, JSON.stringify(addrs, null, 2));
  console.log("✅ source stack deployed + seeded. Outbox:", outboxAddr, "RelayerContract:", rcAddr);
  process.exit(0);
}
main().catch((e) => { console.error("FAILED:", e.shortMessage ?? e.message); process.exit(1); });
