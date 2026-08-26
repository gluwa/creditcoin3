// Plain-ethers source-stack deploy (no hardhat runtime — it wedges on this RPC).
// Reads compiled artifacts from usc-contracts/artifacts. Run with tsx (Node 22).
// Post-#23 redesign: RelayerContract owns verifier/decoder, RelayerFeeVault is a dumb vault,
// AcknowledgmentValidator takes proofVerifier+attestToken, AttestorVault needs an AttestorRegistry
// (mirrors deploy-source-devnet.mts, which already targets this same contract layout).
import { ethers } from "ethers";
import { ApiPromise, Keyring, WsProvider } from "@polkadot/api";
import { cryptoWaitReady } from "@polkadot/util-crypto";
import { readFileSync, writeFileSync } from "node:fs";

// asc-contracts checkout: $ASC_CONTRACTS_DIR, else the sibling of this repo (…/Projects/asc-contracts).
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

async function registerFactoryBeforeOutbox(factory: string) {
  const ws = process.env.CREDITCOIN_SUBSTRATE_WS_URL ?? "ws://127.0.0.1:9944";
  const api = await ApiPromise.create({ provider: new WsProvider(ws), noInitWarn: true });
  try {
    await api.isReady;
    await cryptoWaitReady();
    const sudo = new Keyring({ type: "sr25519" }).addFromUri("//Alice");
    console.log(`  registering OutboxFactory ${factory} before Outbox creation…`);
    await new Promise<void>((resolve, reject) => {
      let unsubscribe: (() => void) | undefined;
      void api.tx.sudo
        .sudo(api.tx.supportedChains.setOutboxFactoryAddr(CHAIN_KEY, factory))
        .signAndSend(sudo, ({ status, dispatchError }) => {
          if (dispatchError) {
            unsubscribe?.();
            reject(new Error(dispatchError.toString()));
          } else if (status.isInBlock || status.isFinalized) {
            unsubscribe?.();
            resolve();
          }
        })
        .then((unsub) => {
          unsubscribe = unsub;
        })
        .catch(reject);
    });
    const registered = await api.query.supportedChains.outboxFactories(CHAIN_KEY);
    const registeredAddress = registered.isSome ? registered.unwrap().toString() : "none";
    if (registeredAddress.toLowerCase() !== factory.toLowerCase()) {
      throw new Error(`factory registration did not land: expected ${factory}, got ${registeredAddress}`);
    }
    console.log("  OutboxFactory governance registration confirmed on-chain");
  } finally {
    await api.disconnect();
  }
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
  // Registry starts empty; owner (this deployer) can updateAttestorSet later if the vault flow needs it.
  const registry = await deploy("AttestorRegistry", ART("write-ability/AttestorRegistry.sol", "AttestorRegistry"), [owner, []]);
  const factory = await deploy("OutboxFactory", ART("write-ability/deployer/OutboxFactory.sol", "OutboxFactory"));
  // Indexer discovery is fail-closed: authenticate OutboxCreated against governance registration.
  await registerFactoryBeforeOutbox(await factory.getAddress());
  const quoter = await deploy("USCRelayingQuoter", ART("write-ability/USCRelayingQuoter.sol", "USCRelayingQuoter"), [owner, await twap.getAddress(), owner]);
  // Outbox.coreFee() reads through IFeeRegistry, NOT the quoter — FeeRegistry wraps this chain's
  // own chain-info precompile (ICoreFeeProvider.get_core_fee(uint32), selector 0x5b023376, fixed
  // address AddressU64<4051> per runtime/src/precompiles.rs). An unconfigured chain_key reads back
  // 0 (no fee), which is fine for the e2e — it just means outbox.coreFee() doesn't revert.
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

  // Acknowledgment round-trip: deploy the AcknowledgmentValidator for the destination chain key,
  // make it the Outbox's validator (only it may call acknowledgeMessage), then point it at the
  // Outbox. The relayer's ack submitter proves MessageDelivered on the destination and calls
  // submitAcknowledgment here, which flips messages[id].acknowledged = true on the Outbox.
  const ackValidator = await deploy("AcknowledgmentValidator",
    ART("write-ability/AcknowledgementValidator.sol", "AcknowledgmentValidator"), [CHAIN_KEY, owner, proofAddr, attestAddr]);
  const ackAddr = await ackValidator.getAddress();
  await (await outbox.setValidator(ackAddr)).wait();
  await (await (ackValidator as any).setOutbox(outboxAddr)).wait();
  // Without this, trustedInboxes[dest.inbox] stays false and submitAcknowledgment skips every
  // MessageDelivered log it decodes, reverting NoMessageDeliveredLogs even with a valid proof —
  // the decoder's own setTrustedInbox below is a separate allowlist and does not cover this one.
  await (await (ackValidator as any).updateTrustedInbox(addrs.dest.inbox, true)).wait();
  console.log("  ack: validator", ackAddr, "→ Outbox, trusted inbox", addrs.dest.inbox);
  await (await (rc as any).setRelayerFeeVault(fvAddr)).wait();
  await (await (rc as any).setDestinationEvmChainId(CHAIN_KEY, DEST_CHAIN_ID)).wait();
  await (await (quoter as any).addQuoter(QUOTER_EOA)).wait();
  await (await (quoter as any).setCoreFee(CHAIN_KEY, ethers.parseEther("1"))).wait();
  await (await (decoder as any).setTrustedInbox(DEST_CHAIN_ID, addrs.dest.inbox)).wait();
  console.log("  wired: forwarder, feeVault, destEvmChainId, quoter EOA, coreFee, trustedInbox");

  // seed prices (deployer == oracleService)
  await (await (twap as any).update(ethers.parseEther("1"))).wait();
  await (await (quoter as any).setPricingMode(0, 10_000_000_000n)).wait();
  await (await (quoter as any).priceUpdate(10_000_000_000n, CHAIN_KEY, [0n, 1n, 10_000_000_000n, 10_000_000_000n, 0])).wait();
  console.log("  seeded quoter prices");

  addrs.source = {
    chainId: 42, rpc: "http://127.0.0.1:9944", chainKey: CHAIN_KEY,
    attest: attestAddr, proofVerifier: proofAddr, deliveryDecoder: decoderAddr,
    twapReader: await twap.getAddress(), attestorRegistry: await registry.getAddress(),
    quoter: quoterAddr, feeRegistry: feeRegistryAddr, factory: await factory.getAddress(),
    attestorVault: avAddr, outbox: outboxAddr, relayerFeeVault: fvAddr, relayerContract: rcAddr,
    ackValidator: ackAddr,
    quoterEOA: QUOTER_EOA, coreFee: ethers.parseEther("1").toString(),
  };
  writeFileSync(OUT, JSON.stringify(addrs, null, 2));
  console.log("✅ source stack deployed + seeded. Outbox:", outboxAddr, "RelayerContract:", rcAddr);
  process.exit(0);
}
main().catch((e) => { console.error("FAILED:", e.shortMessage ?? e.message); process.exit(1); });
