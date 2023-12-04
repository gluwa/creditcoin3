import { ApiPromise } from '@polkadot/api';
import {encodeAddress,decodeAddress, blake2AsHex} from '@polkadot/util-crypto';
import { initEthKeyringPair } from '../account/keyring';
import { JsonRpcProvider, ethers } from 'ethers';

export function evmAddressToSubstrateAddress (evmAddress: string)
{
    const evmAddressBytes = Buffer.from(evmAddress.replace('0x', ''), 'hex');
    const prefixBytes = Buffer.from('evm:', 'utf8');
    const concatBytes = Uint8Array.from(Buffer.concat([prefixBytes, evmAddressBytes]));
    const addressHex = blake2AsHex(concatBytes, 256);
    const substrateAddress = encodeAddress(addressHex);
    return substrateAddress;
}

export function substrateAddressToEvmAddress (substrateAddress: string)
{
    const pubkey = '0x' + Buffer.from(decodeAddress(substrateAddress)).toString('hex');
    const evmAddress = pubkey.slice(0, 42);
    return evmAddress;
}