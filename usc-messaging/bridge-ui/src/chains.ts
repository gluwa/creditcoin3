import { ethers } from "ethers";
import type { ChainConfig } from "./config";
import { ERC20_ABI } from "./abi";

declare global {
  interface Window {
    ethereum?: any;
  }
}

/** Read-only provider. `readRpcPath` is either a same-origin proxy path (`/rpc/cc`, for endpoints
 * without CORS) or an absolute URL hit directly from the browser (for CORS-serving public RPCs
 * like publicnode — proxying those through Vercel gets the proxy's egress IPs 403'd).
 *
 * Public gateways also 403 some *viewer* IPs (geo / VPN / reputation filtering), so when
 * `readRpcFallbacks` is set the reads run through a quorum-1 FallbackProvider that moves down the
 * list whenever the preferred endpoint errors. Static network avoids extra RPC. */
export function readProvider(chain: ChainConfig): ethers.AbstractProvider {
  const network = ethers.Network.from(chain.chainId);
  const mk = (u: string) =>
    new ethers.JsonRpcProvider(new URL(u, location.origin).toString(), network, {
      staticNetwork: true,
    });

  const fallbacks = chain.readRpcFallbacks ?? [];
  if (fallbacks.length === 0) return mk(chain.readRpcPath);

  return new ethers.FallbackProvider(
    [chain.readRpcPath, ...fallbacks].map((u, i) => ({
      provider: mk(u),
      priority: i + 1, // try in config order
      weight: 1,
      stallTimeout: 1500, // hedge to the next endpoint if the preferred one is slow/silent
    })),
    network,
    { quorum: 1 },
  );
}

export function browserProvider(): ethers.BrowserProvider {
  if (!window.ethereum) throw new Error("MetaMask not found — install it and reload.");
  return new ethers.BrowserProvider(window.ethereum);
}

export async function connectWallet(): Promise<string> {
  const accounts: string[] = await window.ethereum.request({ method: "eth_requestAccounts" });
  return ethers.getAddress(accounts[0]);
}

const hex = (n: number) => "0x" + n.toString(16);

/** Switch MetaMask to `chain`, adding it first if unknown. */
export async function ensureChain(chain: ChainConfig): Promise<void> {
  try {
    await window.ethereum.request({
      method: "wallet_switchEthereumChain",
      params: [{ chainId: hex(chain.chainId) }],
    });
  } catch (err: any) {
    // 4902 = chain not added yet.
    if (err?.code === 4902 || /Unrecognized chain/i.test(err?.message ?? "")) {
      await window.ethereum.request({
        method: "wallet_addEthereumChain",
        params: [
          {
            chainId: hex(chain.chainId),
            chainName: chain.name,
            rpcUrls: [chain.metamaskRpcUrl],
            nativeCurrency: { name: chain.currencySymbol, symbol: chain.currencySymbol, decimals: 18 },
          },
        ],
      });
    } else {
      throw err;
    }
  }
}

export async function signerFor(chain: ChainConfig): Promise<ethers.Signer> {
  await ensureChain(chain);
  return browserProvider().getSigner();
}

/** Raw token balance (wei) read off the proxied RPC. */
export async function tokenBalance(chain: ChainConfig, account: string): Promise<bigint> {
  if (!chain.token) return 0n;
  const token = new ethers.Contract(chain.token, ERC20_ABI, readProvider(chain));
  return token.balanceOf(account);
}

export interface ProofResponse {
  headerNumber: number;
  txBytes: string;
  continuityProof: { lowerEndpointDigest: string; roots: string[] };
  merkleProof: { root: string; siblings: { hash: string; isLeft: boolean }[] };
}

/**
 * Poll proof-gen for a native USC proof of `txHash` on `chainKey`. Early in a run the block isn't
 * attested yet (422 BlockNotReady / 404 AttestationsMissing) — both are transient, so we retry.
 * `onTick(attempt)` is called each poll so the UI can animate.
 */
/** A proof-gen error we should NOT retry (bad request, unknown chain, malformed proof). */
class ProofFatal extends Error {}

/** Wait until the destination chain is `confirmations` blocks past `fromBlock` (finality), so
 * proof-gen has a chance of having attested it. Polls the head; `onTick` gets the remaining blocks.
 * Returns after the target height (or a generous safety timeout — proof-gen still gates on the real
 * attestation, so proceeding early just means fetchProof retries a bit longer). */
export async function waitForFinality(
  chain: ChainConfig,
  fromBlock: number,
  confirmations: number,
  onTick: (remainingBlocks: number) => void,
  maxWaitSec = 1200,
): Promise<void> {
  const provider = readProvider(chain);
  const target = fromBlock + confirmations;
  const deadline = Date.now() + maxWaitSec * 1000;
  for (;;) {
    let head = fromBlock;
    try {
      head = await provider.getBlockNumber();
    } catch {
      /* transient RPC hiccup — keep waiting */
    }
    onTick(Math.max(0, target - head));
    if (head >= target || Date.now() > deadline) return;
    await new Promise((r) => setTimeout(r, 4000));
  }
}

export async function fetchProof(
  proofGenPath: string,
  chainKey: number,
  txHash: string,
  onTick: (attempt: number) => void,
  maxAttempts = 300,
): Promise<ProofResponse> {
  const url = `${proofGenPath.replace(/\/$/, "")}/api/v1/proof-by-tx/${chainKey}/${txHash}`;
  let lastErr = "";
  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    onTick(attempt);
    try {
      const resp = await fetch(url);
      if (resp.ok) {
        const proof = (await resp.json()) as ProofResponse;
        if (!proof.txBytes) throw new ProofFatal("proof-gen returned no txBytes (continuity-only proof)");
        return proof;
      }
      const body = await resp.text();
      // Retry while the block isn't attested yet (422 / markers) AND on proxy/server hiccups (5xx,
      // incl. the 502 from the ingress) and network blips — only give up on a clear client error.
      const retryable =
        resp.status === 422 ||
        resp.status >= 500 ||
        body.includes("BlockNotReady") ||
        body.includes("AttestationsMissing");
      if (!retryable) throw new ProofFatal(`proof-gen ${resp.status}: ${body.slice(0, 200)}`);
      lastErr = `proof-gen ${resp.status}`;
    } catch (e) {
      if (e instanceof ProofFatal) throw e; // genuine failure — stop
      lastErr = (e as Error)?.message ?? "network error"; // fetch threw (offline/proxy) — retry
    }
    await new Promise((r) => setTimeout(r, 3000));
  }
  throw new Error(`proof not ready after ${maxAttempts} attempts (last: ${lastErr})`);
}
