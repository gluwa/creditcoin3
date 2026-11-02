import { Mnemonic, Wallet, concat, keccak256, toUtf8Bytes } from "ethers";

// Public BIP39 test vectors. Unlike zombienet's --well-known-keys, these do not include
// //Alice, which the dev runtime rejects as an attestor because it is the sudo account.
export const ATTESTOR_MNEMONICS = [
  "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
  "legal winner thank year wave sausage worth useful legal winner thank yellow",
  "letter advice cage absurd amount doctor acoustic avoid letter advice cage above",
  "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong",
];

// Mirrors AttestorSecret::to_seed_bytes_32 and MessageSigner::from_seed. The scenario
// checks these identities against the running fleet's destination registry before signing.
export const committee = ATTESTOR_MNEMONICS.map((phrase) => {
  const seed = Mnemonic.fromPhrase(phrase).computeSeed().slice(0, 66);
  return new Wallet(
    keccak256(concat([toUtf8Bytes("usc/write-ability/evm-signer/v1"), seed])),
  );
});
