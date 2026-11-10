// Adds a second source Outbox for the same chain key to an existing e2e deployment, for the
// defaultOutbox rotation drill. An Outbox brings its own AttestorVault, RelayerFeeVault,
// RelayerContract and AcknowledgmentValidator (each is bound to one Outbox for life); the proof
// verifier, decoders, quoter contract, fee registry and Discovery are shared with the first.
// The new Outbox is registered in Discovery but does not become the default.
import { ethers } from "ethers";
import { readFileSync, writeFileSync } from "node:fs";

const dir = process.env.ASC_CONTRACTS_DIR;
if (!dir) throw new Error("set ASC_CONTRACTS_DIR to a compiled asc-contracts");
const ART = (p: string, n: string) =>
  JSON.parse(readFileSync(`${dir}/artifacts/contracts/${p}/${n}.json`, "utf8"));
const OUT = process.env.E2E_DEPLOY_PATH;
if (!OUT) throw new Error("set E2E_DEPLOY_PATH");
const DEAD = "0x000000000000000000000000000000000000dEaD";
const ANVIL0 =
  "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

const addrs = JSON.parse(readFileSync(OUT, "utf8"));
const s = addrs.source;
const provider = new ethers.JsonRpcProvider(
  process.env.CREDITCOIN_RPC_URL ?? s.rpc,
  s.chainId,
  { staticNetwork: true, polling: true, cacheTimeout: -1 },
);
provider.pollingInterval = 800;
const wallet = new ethers.Wallet(
  process.env.CREDITCOIN_CHAIN_PRIVATE_KEY!,
  provider,
);
const owner = wallet.address;

async function deploy(name: string, art: any, args: unknown[]) {
  const contract = await new ethers.ContractFactory(
    art.abi,
    art.bytecode,
    wallet,
  ).deploy(...args);
  await contract.waitForDeployment();
  console.log(`  ${name} → ${await contract.getAddress()}`);
  return contract as any;
}
const at = (art: any, address: string) =>
  new ethers.Contract(address, art.abi, wallet) as any;

const factory = at(
  ART("write-ability/deployer/OutboxFactory.sol", "OutboxFactory"),
  s.factory,
);
const discovery = at(
  ART("write-ability/deployer/OutboxDiscovery.sol", "OutboxDiscovery"),
  s.outboxDiscovery,
);

// av (nonce n), fv (n+1), rc (n+2); the Outbox address comes from CREATE2 over the vault address.
const n = await wallet.getNonce();
const [avAddr, fvAddr, rcAddr] = [0, 1, 2].map((k) =>
  ethers.getCreateAddress({ from: owner, nonce: n + k }),
);
const outboxArgs = [owner, owner, 0n, avAddr, s.feeRegistry, s.attest];
const obAddr: string = await factory.computeOutboxAddressFor(
  owner,
  s.chainKey,
  ...outboxArgs,
);
if (obAddr.toLowerCase() === s.outbox.toLowerCase())
  throw new Error("second Outbox address collides with the first");
await deploy(
  "AttestorVault",
  ART("write-ability/AttestorVault.sol", "AttestorVault"),
  [owner, s.attest, obAddr, rcAddr, owner, s.attestorRegistry, DEAD, 0],
);
await deploy(
  "RelayerFeeVault",
  ART("write-ability/RelayerFeeVault.sol", "RelayerFeeVault"),
  [s.attest, rcAddr],
);
const rc = await deploy(
  "RelayerContract",
  ART("write-ability/RelayerContract.sol", "RelayerContract"),
  [owner, s.attest, obAddr, s.quoter, s.proofVerifier, s.deliveryDecoder],
);
if ((await rc.getAddress()) !== rcAddr) throw new Error("precompute mismatch");
await (await factory.deployOutbox(s.chainKey, ...outboxArgs)).wait();
const outbox = at(ART("write-ability/Outbox.sol", "Outbox"), obAddr);
if ((await provider.getCode(obAddr)) === "0x")
  throw new Error("second Outbox was not created at its CREATE2 address");
console.log("  Outbox →", obAddr);

await (await discovery.registerOutbox(s.chainKey, obAddr)).wait();
const current = (await discovery.defaultOutbox(s.chainKey)) as string;
if (
  current.toLowerCase() !== s.outbox.toLowerCase() ||
  !(await discovery.isActiveOutbox(s.chainKey, obAddr))
)
  throw new Error("registering the second Outbox must not move the default");

await (await outbox.setTrustedForwarder(rcAddr, true)).wait();
const ack = await deploy(
  "AcknowledgmentValidator",
  ART("write-ability/AcknowledgementValidator.sol", "AcknowledgmentValidator"),
  [s.chainKey, owner, s.proofVerifier, s.attest, s.evmDecoder],
);
const ackAddr = await ack.getAddress();
await (await outbox.setValidator(ackAddr)).wait();
await (await ack.setOutbox(obAddr)).wait();
await (await ack.updateTrustedInbox(addrs.dest.inbox, true)).wait();
await (await rc.setRelayerFeeVault(fvAddr)).wait();
await (
  await rc.setDestinationEvmChainId(s.chainKey, addrs.dest.chainId ?? 31337)
).wait();

const destProvider = new ethers.JsonRpcProvider(
  process.env.DESTINATION_CHAIN_RPC_URL ?? addrs.dest.rpc,
  addrs.dest.chainId ?? 31337,
  { staticNetwork: true, cacheTimeout: -1 },
);
destProvider.pollingInterval = 500;
const inbox = new ethers.Contract(
  addrs.dest.inbox,
  ART("write-ability/Inbox.sol", "Inbox").abi,
  new ethers.Wallet(ANVIL0, destProvider),
) as any;
await (await inbox.setSupportedOutbox(obAddr, true)).wait();
if (!(await inbox.isSupportedOutbox(obAddr)))
  throw new Error("Inbox.setSupportedOutbox did not land");
destProvider.destroy();

// The drill's publishers use their own keys so their nonces never race the relayer's
// settlement signer, which shares the deployer key.
for (const publisher of (process.env.E2E_PUBLISHER_ADDRESSES ?? "")
  .split(",")
  .filter(Boolean)) {
  await (
    await wallet.sendTransaction({
      to: publisher,
      value: ethers.parseEther("1000"),
    })
  ).wait();
}

s.second = {
  outbox: obAddr,
  ackValidator: ackAddr,
  relayerContract: rcAddr,
  relayerFeeVault: fvAddr,
  attestorVault: avAddr,
};
writeFileSync(OUT, JSON.stringify(addrs, null, 2));
console.log("✅ second Outbox deployed and registered:", s.second);
provider.destroy();
