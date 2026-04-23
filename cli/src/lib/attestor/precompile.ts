import { ethers, HDNodeWallet, JsonRpcProvider } from 'ethers';
import { decodeAddress } from '@polkadot/util-crypto';
import { u8aToHex } from '@polkadot/util';
import { OptionValues } from 'commander';
import { evmAddressToSubstrateAddress } from '../evm/address';
import { getEvmUrl } from '../evm/rpc';

// eslint-disable-next-line @typescript-eslint/no-require-imports
import contractABIJSON = require('../../test/blockchain-tests/artifacts/attestor_stash.json');
const contractABI = contractABIJSON as unknown as ethers.InterfaceAbi;

export const ATTESTOR_STASH_ADDRESS = '0x0000000000000000000000000000000000000fd4';

// AttestorStatus: Active = 0, Idle = 1, Waiting = 2.
// Declared as `bigint` constants; call sites MUST `BigInt(info.status)`-coerce
// the struct field before comparing, because ethers.js can surface small
// Solidity integer fields (uint8 here) as plain `number` rather than `bigint`,
// and `0n === 0` is false.
export const ATTESTOR_STATUS_ACTIVE = 0n;
export const ATTESTOR_STATUS_IDLE = 1n;
export const ATTESTOR_STATUS_WAITING = 2n;

/**
 * Convert a Substrate SS58 address to a bytes32 hex string (the raw 32-byte AccountId).
 */
export function substrateAddressToBytes32(ss58: string): string {
    return u8aToHex(decodeAddress(ss58));
}

/**
 * Derive an EVM private key (hex string) and Ethereum address from a BIP39 mnemonic.
 * Uses the standard Ethereum HD path (m/44'/60'/0'/0/0) via ethers.js.
 */
export function deriveEvmKeyFromSecret(secret: string): {
    privateKey: string;
    evmAddress: string;
    stashAddress: string;
} {
    const wallet: HDNodeWallet = HDNodeWallet.fromPhrase(secret);
    const privateKey = wallet.privateKey;
    const evmAddress = wallet.address;
    const stashAddress = evmAddressToSubstrateAddress(evmAddress);
    return { privateKey, evmAddress, stashAddress };
}

/**
 * Create an ethers.js Contract instance for the attestor-stash precompile,
 * backed by a wallet derived from the CLI secret.
 * The EVM URL is derived from options.url (ws → http).
 */
export function getAttestorContractWithSigner(
    secret: string,
    options: OptionValues,
): { contract: ethers.Contract; provider: JsonRpcProvider; wallet: ethers.Wallet; stashAddress: string } {
    const evmUrl = getEvmUrl(options);
    const provider = new JsonRpcProvider(evmUrl);
    const { privateKey, stashAddress } = deriveEvmKeyFromSecret(secret);
    const wallet = new ethers.Wallet(privateKey, provider);
    const contract = new ethers.Contract(ATTESTOR_STASH_ADDRESS, contractABI, wallet);
    return { contract, provider, wallet, stashAddress };
}

/**
 * Create a read-only ethers.js Contract instance for the attestor-stash precompile.
 * No wallet needed; only view functions can be called.
 */
export function getAttestorContractReadOnly(options: OptionValues): ethers.Contract {
    const evmUrl = getEvmUrl(options);
    const provider = new JsonRpcProvider(evmUrl);
    return new ethers.Contract(ATTESTOR_STASH_ADDRESS, contractABI, provider);
}

/**
 * Extract a human-readable error message from an EVM revert.
 *
 * Frontier's `try_dispatch` wraps the underlying `DispatchError` in the EVM
 * revert reason using its `Debug` representation, e.g.:
 *
 *     Module(ModuleError { index: 25, error: [20, 0, 0, 0], message: Some("InsufficientBalance") })
 *
 * When ethers.js surfaces this through an error message, the inner quotes may
 * or may not be backslash-escaped depending on where we pick up the string.
 * We try, in order:
 *   1. The pallet error variant name inside `message: Some("…")` (both the
 *      plain and backslash-escaped forms).
 *   2. The full `reason="…"` revert string.
 *   3. The raw error message as a fallback.
 */
export function extractEvmError(error: unknown): string {
    if (error instanceof Error) {
        const msg = error.message;
        // Pallet error name — handles both `Some("X")` and `Some(\"X\")` forms.
        const palletMatch = msg.match(/message:\s*Some\(\\?"([A-Za-z0-9_]+)\\?"\)/);
        if (palletMatch) {
            return `Transaction failed with error: "${palletMatch[1]}"`;
        }
        // Full revert reason — allow escaped quotes inside the captured group.
        const revertMatch = msg.match(/reason="((?:\\.|[^"\\])*)"/);
        if (revertMatch) {
            return `Transaction failed with error: "${revertMatch[1]}"`;
        }
        return `Transaction failed: ${msg}`;
    }
    return `Transaction failed: ${String(error)}`;
}
