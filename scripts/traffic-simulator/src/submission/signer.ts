/**
 * Signer cache and submission lock for sequential precompile transactions.
 */

import { ethers, JsonRpcProvider } from "ethers";

export type SignerEntry = {
  signer: ethers.NonceManager;
  provider: JsonRpcProvider;
};

const signerCache = new Map<string, SignerEntry>();
let submissionQueue: Promise<void> = Promise.resolve();

export function getSigner(
  cc3HttpUrl: string,
  privateKey: string,
): SignerEntry {
  const key = `${cc3HttpUrl}:${privateKey}`;
  let entry = signerCache.get(key);
  if (!entry) {
    const provider = new ethers.JsonRpcProvider(cc3HttpUrl);
    const wallet = new ethers.Wallet(privateKey, provider);
    entry = { signer: new ethers.NonceManager(wallet), provider };
    signerCache.set(key, entry);
  }
  return entry;
}

export function resetSigner(
  cc3HttpUrl: string,
  privateKey: string,
): SignerEntry {
  const key = `${cc3HttpUrl}:${privateKey}`;
  const old = signerCache.get(key);
  signerCache.delete(key);
  // Best-effort cleanup of the abandoned provider. `destroy()` clears
  // internal subscribers and pollers; it does NOT abort in-flight HTTP
  // requests (ethers v6 doesn't expose a way to cancel them), so any
  // late `request timeout` rejection still has to be swallowed by
  // withTimeout's `.catch` and the global handler in main.ts.
  if (old) {
    try {
      old.provider.destroy();
    } catch { /* ignore */ }
  }
  return getSigner(cc3HttpUrl, privateKey);
}

export async function withSubmissionLock<T>(
  fn: () => Promise<T>,
): Promise<T> {
  const previous = submissionQueue;
  let release: () => void;
  submissionQueue = new Promise((r) => (release = r));
  await previous;
  try {
    return await fn();
  } finally {
    release!();
  }
}
