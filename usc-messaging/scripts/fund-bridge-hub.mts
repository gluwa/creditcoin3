// Funds BridgeHub with ATTEST and approves a destination Outbox to pull the FeeRegistry's coreFee
// from it.
//
// Outbox.publishMessage (called internally by BridgeHub.claim, to send the release message once a
// deposit is proven) always pulls `feeRegistry.coreFee(destChainKey)` in ATTEST from `msg.sender` —
// which, when BridgeHub calls it, is BridgeHub itself — whenever that fee is non-zero. Reusing an
// existing FeeRegistry (deploy-bridge-hub.mts does this deliberately, see its own comment) can carry
// a real, already-configured non-zero coreFee for a chain key from before this bridge existed. Until
// BridgeHub is funded and has approved that Outbox, every claim() for that destination reverts with
// CompatibleERC20's ERC20CallFailed(attestToken, transferFrom-selector) — this looks like a generic
// on-chain failure (ethers reports it as "missing revert data"/"unknown custom error") but is really
// just BridgeHub needing an ATTEST balance and allowance, exactly like `approveOutboxFee`'s own
// NatSpec in BridgeHub.sol anticipates.
//
// Usage: tsx fund-bridge-hub.mts <chainKey>
// Env: CC_RPC (default devnet), DEPLOYER_KEY (must be BridgeHub's owner — approveOutboxFee is
//      onlyOwner — and pays for the ATTEST transfer), DEPLOY_OUT (default ./usc-bridge-deploy.json),
//      FUND_AMOUNT (raw ATTEST units to transfer to BridgeHub, default 0 = skip the transfer, e.g.
//      to top up an already-funded hub), APPROVE_AMOUNT (raw ATTEST units to approve the
//      destination Outbox for, default: max uint256 — approve once, never repeat for this outbox).
import { ethers } from "ethers";
import { readFileSync } from "node:fs";

const OUT = process.env.DEPLOY_OUT ?? "./usc-bridge-deploy.json";
const CC_CHAIN_ID = Number(process.env.CREDITCOIN_CHAIN_ID ?? 42);
const ccRpc = process.env.CC_RPC ?? "https://rpc.usc-devnet.creditcoin.network";
const key = process.env.DEPLOYER_KEY!;
if (!key) throw new Error("need DEPLOYER_KEY");

const chainKeyArg = process.argv[2];
if (!chainKeyArg) throw new Error("usage: fund-bridge-hub.mts <chainKey>");
const CHAIN_KEY = Number(chainKeyArg);
if (!Number.isInteger(CHAIN_KEY) || CHAIN_KEY <= 0) {
  throw new Error(`chainKey must be a positive integer, got ${chainKeyArg}`);
}

const fundAmount = BigInt(process.env.FUND_AMOUNT ?? "0");
const approveAmount = process.env.APPROVE_AMOUNT
  ? BigInt(process.env.APPROVE_AMOUNT)
  : ethers.MaxUint256;

const provider = new ethers.JsonRpcProvider(ccRpc, CC_CHAIN_ID, {
  staticNetwork: true,
  polling: true,
});
const wallet = new ethers.Wallet(key, provider);

const ATTEST_ABI = [
  "function transfer(address to, uint256 amount) returns (bool)",
  "function balanceOf(address) view returns (uint256)",
  "function allowance(address,address) view returns (uint256)",
  "function decimals() view returns (uint8)",
  "function symbol() view returns (string)",
];
const OUTBOX_ABI = ["function coreFee() view returns (uint256)"];
const HUB_ABI = [
  "function owner() view returns (address)",
  "function approveOutboxFee(address outbox, uint256 amount)",
];

// Known custom errors that can surface from either call below, so a revert prints a readable
// reason instead of ethers' generic "execution reverted (unknown custom error)" — that message
// means ethers has no ABI for whatever four-byte selector came back, not that there's no reason at
// all. ERC20InsufficientBalance/Allowance are OpenZeppelin v5's standard ERC20 (what ATTEST uses);
// ERC20CallFailed is CompatibleERC20's wrapper (used by Outbox.publishMessage internally, so it can
// only surface here if you're pointed at the wrong attestToken address); OwnableUnauthorizedAccount
// is the Ownable2Step error approveOutboxFee's onlyOwner would throw if this raced a real ownership
// change after the explicit owner() check above.
const KNOWN_ERRORS = new ethers.Interface([
  "error ERC20InsufficientBalance(address sender, uint256 balance, uint256 needed)",
  "error ERC20InsufficientAllowance(address spender, uint256 allowance, uint256 needed)",
  "error ERC20CallFailed(address token, bytes4 selector)",
  "error OwnableUnauthorizedAccount(address account)",
]);

function describeError(err: unknown): string {
  const data =
    (err as { data?: string })?.data ??
    (err as { info?: { error?: { data?: string } } })?.info?.error?.data;
  if (!data) return (err as Error).message ?? String(err);
  try {
    const parsed = KNOWN_ERRORS.parseError(data);
    if (parsed) return `${parsed.name}(${parsed.args.map(String).join(", ")})`;
  } catch {
    // fall through — an error we don't have the ABI for
  }
  return `${(err as Error).message ?? String(err)} (raw revert data: ${data})`;
}

async function main() {
  const out = JSON.parse(readFileSync(OUT, "utf8"));
  const hubAddr = out.hub?.bridgeHub as string | undefined;
  const attestAddr = out.hub?.attestToken as string | undefined;
  const outboxAddr = out.spokes?.[CHAIN_KEY]?.sharedOutbox as
    | string
    | undefined;
  if (!hubAddr || !attestAddr) {
    throw new Error(
      `${OUT}: hub.bridgeHub/hub.attestToken missing — run deploy-bridge-hub.mts first`,
    );
  }
  if (!outboxAddr) {
    throw new Error(
      `${OUT}: spokes.${CHAIN_KEY}.sharedOutbox missing — run deploy-bridge-spoke.mts ${CHAIN_KEY} first`,
    );
  }

  const attest = new ethers.Contract(attestAddr, ATTEST_ABI, wallet);
  const outbox = new ethers.Contract(outboxAddr, OUTBOX_ABI, provider);
  const hub = new ethers.Contract(hubAddr, HUB_ABI, wallet);

  const hubOwner = await hub.owner();
  if (hubOwner.toLowerCase() !== wallet.address.toLowerCase()) {
    throw new Error(
      `DEPLOYER_KEY (${wallet.address}) is not BridgeHub's owner (${hubOwner}) — ` +
        "approveOutboxFee is onlyOwner",
    );
  }

  const [decimals, symbol] = await Promise.all([
    attest.decimals().catch(() => 18),
    attest.symbol().catch(() => "ATTEST"),
  ]);
  const fmt = (v: bigint) =>
    `${v} (${ethers.formatUnits(v, decimals)} ${symbol})`;

  const coreFee: bigint = await outbox.coreFee();
  const walletBalance: bigint = await attest.balanceOf(wallet.address);
  const balanceBefore: bigint = await attest.balanceOf(hubAddr);
  const allowanceBefore: bigint = await attest.allowance(hubAddr, outboxAddr);
  console.log(`chainKey=${CHAIN_KEY} outbox=${outboxAddr}`);
  console.log(`coreFee: ${fmt(coreFee)}`);
  console.log(
    `signer ${wallet.address} ${symbol} balance: ${fmt(walletBalance)}`,
  );
  console.log(
    `BridgeHub ATTEST balance: ${fmt(balanceBefore)}, allowance to outbox: ${fmt(allowanceBefore)}`,
  );
  if (coreFee === 0n) {
    console.log(
      "coreFee is 0 — BridgeHub.claim won't need to pull any ATTEST for this chain key.",
    );
  }
  if (fundAmount > 0n && fundAmount > walletBalance) {
    throw new Error(
      `FUND_AMOUNT (${fmt(fundAmount)}) exceeds the signer's own balance (${fmt(walletBalance)}) — ` +
        "lower FUND_AMOUNT or fund the signer wallet first",
    );
  }

  // Explicit nonces for this back-to-back pair — see deploy-bridge-spoke.mts's comment on ethers
  // v6's nonce-caching gotcha against fast-mining chains.
  const n = await wallet.getNonce();
  let nonce = n;

  if (fundAmount > 0n) {
    console.log(`transferring ${fmt(fundAmount)} to BridgeHub...`);
    try {
      const tx = await attest.transfer(hubAddr, fundAmount, { nonce: nonce++ });
      console.log(`  tx ${tx.hash}, waiting for it to be mined...`);
      const receipt = await tx.wait();
      console.log(`  mined in block ${receipt.blockNumber}`);
    } catch (err) {
      throw new Error(
        `ATTEST transfer to BridgeHub failed: ${describeError(err)}`,
      );
    }
  }

  console.log(`approving outbox ${outboxAddr} for ${fmt(approveAmount)}...`);
  try {
    const tx = await hub.approveOutboxFee(outboxAddr, approveAmount, {
      nonce: nonce++,
    });
    console.log(`  tx ${tx.hash}, waiting for it to be mined...`);
    const receipt = await tx.wait();
    console.log(`  mined in block ${receipt.blockNumber}`);
  } catch (err) {
    throw new Error(`approveOutboxFee failed: ${describeError(err)}`);
  }

  const balanceAfter: bigint = await attest.balanceOf(hubAddr);
  const allowanceAfter: bigint = await attest.allowance(hubAddr, outboxAddr);
  console.log(
    `BridgeHub ATTEST balance: ${fmt(balanceAfter)}, allowance to outbox: ${fmt(allowanceAfter)}`,
  );
  if (coreFee > 0n) {
    console.log(
      `  covers ${balanceAfter / coreFee} more claim(s) at the current coreFee`,
    );
  }
  console.log("✅ done");
  process.exit(0);
}
main().catch((e) => {
  console.error("FAILED:", e.shortMessage ?? e.message ?? e);
  if (e.reason) console.error("  reason:", e.reason);
  if (e.code) console.error("  code:", e.code);
  if (e.info) console.error("  info:", JSON.stringify(e.info, null, 2));
  process.exit(1);
});
