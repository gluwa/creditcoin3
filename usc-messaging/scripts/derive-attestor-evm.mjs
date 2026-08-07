#!/usr/bin/env node
// Derive each attestor's USC write-ability EVM vote address from its SECRET (BIP39 mnemonic or a
// 0x 32-byte hex seed). Mirrors the attestor's signing.rs exactly:
//   seed32  = mnemonic ? BIP39_seed(phrase,"")[..32] : rawSeed
//   privkey = keccak256("usc/write-ability/evm-signer/v1" || seed32)
//   address = EVM address of that secp256k1 key
//
// NOTE: you CANNOT derive this from an SS58 address — the EVM key comes from the seed, and SS58 is a
// one-way hash of a different (sr25519) key. Feed the SECRETS, not the SS58 list.
//
// Usage:
//   node scripts/derive-attestor-evm.mjs secrets.txt       # one mnemonic / 0x-seed per line
//   printf '%s\n' "word1 word2 ... word12" | node scripts/derive-attestor-evm.mjs
import { Mnemonic, Wallet, keccak256, concat, toUtf8Bytes, getBytes } from "ethers";
import { readFileSync } from "node:fs";

const DOMAIN = toUtf8Bytes("usc/write-ability/evm-signer/v1");

function seed32(secret) {
  const s = secret.trim();
  if (/^0x[0-9a-fA-F]{64}$/.test(s)) return getBytes(s); // raw 32-byte hex seed
  return getBytes(Mnemonic.fromPhrase(s).computeSeed()).slice(0, 32); // BIP39 64-byte seed -> [..32]
}

function evmAddress(secret) {
  const priv = keccak256(concat([DOMAIN, seed32(secret)]));
  return new Wallet(priv).address;
}

const raw = process.argv[2] ? readFileSync(process.argv[2], "utf8") : readFileSync(0, "utf8");
const secrets = raw
  .split("\n")
  .map((l) => l.trim())
  .filter((l) => l && !l.startsWith("#"));

const addrs = secrets.map(evmAddress);
addrs.forEach((a, i) => console.log(`${String(i).padStart(2)}: ${a}`));
console.log("\ncomma-separated (updateAttestorSet / --attestor-set):");
console.log(addrs.join(","));
