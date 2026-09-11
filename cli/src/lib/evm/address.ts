import { encodeAddress, decodeAddress, blake2AsHex, blake2AsU8a } from '@polkadot/util-crypto';
import { getBytes } from 'ethers';

export function evmAddressToSubstrateAddress(evmAddress: string) {
    const evmAddressBytes = Buffer.from(evmAddress.replace('0x', ''), 'hex');
    const prefixBytes = Buffer.from('evm:', 'utf8');
    const concatBytes = Uint8Array.from(Buffer.concat([prefixBytes, evmAddressBytes]));
    const addressHex = blake2AsHex(concatBytes, 256);
    const substrateAddress = encodeAddress(addressHex);
    return substrateAddress;
}

export function substrateAddressToEvmAddress(substrateAddress: string) {
    const pubkey = '0x' + Buffer.from(decodeAddress(substrateAddress)).toString('hex');
    const evmAddress = pubkey.slice(0, 42);
    return evmAddress;
}

/**
 * Convert an EVM address to the raw 32-byte Substrate AccountId produced by
 * `pallet_evm::HashedAddressMapping::<BlakeTwo256>::into_account_id`.
 *
 * Layout: blake2_256("evm:" || 20-byte EVM address)
 */
export function evmAddressToSubstrateAccountId(evmAddress: string): Uint8Array {
    const addr = getBytes(evmAddress);
    const payload = new Uint8Array(24);
    payload.set(new TextEncoder().encode('evm:'), 0);
    payload.set(addr, 4);
    return blake2AsU8a(payload, 256);
}
