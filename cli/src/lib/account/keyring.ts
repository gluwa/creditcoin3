import { Keyring } from '@polkadot/keyring';

export function initKeyringPair(seed: string) {
    const keyring = new Keyring({ type: 'sr25519' });
    const pair = keyring.addFromUri(`${seed}`);
    return pair;
}
export function initECDSAKeyringPairFromPK(pk: string) {
    const keyring = new Keyring({ type: 'ecdsa' });
    const pair = keyring.addFromUri(`${pk}`);
    return pair;
}

export function initEthKeyringPair(seed: string, accIndex = 0) {
    const keyring = new Keyring({ type: 'ethereum' });
    const pair = keyring.addFromUri(`${seed}/m/44'/60'/0'/0/${accIndex}`);
    return pair;
}
