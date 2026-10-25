/**
 * Generates invalid API requests across multiple error categories.
 *
 * Categories:
 * - wrongChainKey: valid format but wrong chain_key value
 * - nonExistentBlock: block number far beyond current height
 * - blockBeforeGenesis: block 0 or 1
 * - txIndexOutOfBounds: absurdly high tx index
 * - malformedTxHash: bad hex, wrong length, invalid chars
 * - nonExistentTxHash: valid format but random bytes
 * - invalidPathParams: strings where numbers expected
 */

import type { StressRequest } from "../types.ts";

type InvalidCategory =
  | "wrongChainKey"
  | "nonExistentBlock"
  | "blockBeforeGenesis"
  | "txIndexOutOfBounds"
  | "malformedTxHash"
  | "nonExistentTxHash"
  | "invalidPathParams";

const ALL_CATEGORIES: InvalidCategory[] = [
  "wrongChainKey",
  "nonExistentBlock",
  "blockBeforeGenesis",
  "txIndexOutOfBounds",
  "malformedTxHash",
  "nonExistentTxHash",
  "invalidPathParams",
];

function randomHex(bytes: number): string {
  const arr = new Uint8Array(bytes);
  crypto.getRandomValues(arr);
  return "0x" +
    Array.from(arr, (b) => b.toString(16).padStart(2, "0")).join("");
}

function generateOne(
  apiUrl: string,
  chainKey: number,
  category: InvalidCategory,
): StressRequest {
  switch (category) {
    case "wrongChainKey": {
      const badKeys = [chainKey + 1, 0, 999999, chainKey + 100];
      const badKey = badKeys[Math.floor(Math.random() * badKeys.length)];
      return {
        url: `${apiUrl}/api/v1/proof/${badKey}/1000/0`,
        kind: "invalid",
        invalidCategory: "wrongChainKey",
      };
    }

    case "nonExistentBlock": {
      const futureBlock = 999_999_999 +
        Math.floor(Math.random() * 1_000_000);
      return {
        url: `${apiUrl}/api/v1/proof/${chainKey}/${futureBlock}/0`,
        kind: "invalid",
        invalidCategory: "nonExistentBlock",
      };
    }

    case "blockBeforeGenesis": {
      const earlyBlock = Math.floor(Math.random() * 2);
      return {
        url: `${apiUrl}/api/v1/proof/${chainKey}/${earlyBlock}/0`,
        kind: "invalid",
        invalidCategory: "blockBeforeGenesis",
      };
    }

    case "txIndexOutOfBounds": {
      const hugeTxIndex = 999_999 + Math.floor(Math.random() * 1_000_000);
      return {
        url: `${apiUrl}/api/v1/proof/${chainKey}/1000/${hugeTxIndex}`,
        kind: "invalid",
        invalidCategory: "txIndexOutOfBounds",
      };
    }

    case "malformedTxHash": {
      const malformed = [
        "0xNOTHEX",
        "0x1234",
        "0x" + "ff".repeat(40),
        "zzzzzz",
        "0xGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG",
        "",
      ];
      const bad = malformed[Math.floor(Math.random() * malformed.length)];
      return {
        url: `${apiUrl}/api/v1/proof-by-tx/${chainKey}/${bad}`,
        kind: "invalid",
        invalidCategory: "malformedTxHash",
      };
    }

    case "nonExistentTxHash": {
      const fakeHash = randomHex(32);
      return {
        url: `${apiUrl}/api/v1/proof-by-tx/${chainKey}/${fakeHash}`,
        kind: "invalid",
        invalidCategory: "nonExistentTxHash",
      };
    }

    case "invalidPathParams": {
      const badPaths = [
        `${apiUrl}/api/v1/proof/${chainKey}/abc/0`,
        `${apiUrl}/api/v1/proof/${chainKey}/1000/xyz`,
        `${apiUrl}/api/v1/proof/notanumber/1000/0`,
        `${apiUrl}/api/v1/proof-by-tx/${chainKey}/`,
        `${apiUrl}/api/v1/proof/${chainKey}/-1/0`,
      ];
      const bad = badPaths[Math.floor(Math.random() * badPaths.length)];
      return {
        url: bad,
        kind: "invalid",
        invalidCategory: "invalidPathParams",
      };
    }
  }
}

/**
 * Generate a pool of invalid requests, evenly distributed across categories.
 */
export function generateInvalidRequests(
  apiUrl: string,
  chainKey: number,
  count: number,
): StressRequest[] {
  const requests: StressRequest[] = [];

  for (let i = 0; i < count; i++) {
    const category = ALL_CATEGORIES[i % ALL_CATEGORIES.length];
    requests.push(generateOne(apiUrl, chainKey, category));
  }

  // Shuffle
  for (let i = requests.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [requests[i], requests[j]] = [requests[j], requests[i]];
  }

  return requests;
}
