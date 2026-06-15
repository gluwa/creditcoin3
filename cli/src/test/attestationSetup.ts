import { KeyringPair, mnemonicGenerate } from '../lib';
import { initKeyringPair } from '../lib/account/keyring';

const createSigner = (who: 'alice' | 'bob' | 'random' | 'sudo'): KeyringPair => {
    switch (who) {
        case 'alice':
            return initKeyringPair('//Alice');
        case 'bob':
            return initKeyringPair('//Bob');
        case 'random':
            const secret = mnemonicGenerate();
            return initKeyringPair(secret);
        case 'sudo':
            return initKeyringPair('//Alice');
        default:
            throw new Error(`Unexpected value "${who}"`); // eslint-disable-line
    }
};

export const creditcoinApiUrl = (defaultUrl: string) => {
    return process.env.CREDITCOIN_API_URL || defaultUrl;
};

const setup = () => {
    process.env.NODE_ENV = 'test';

    if ((global as any).CREDITCOIN_CREATE_SIGNER === undefined) {
        (global as any).CREDITCOIN_CREATE_SIGNER = createSigner; // eslint-disable-line
    }

    if ((global as any).CREDITCOIN_API_URL === undefined) {
        const wsPort = process.env.CREDITCOIN_WS_PORT || '9944';
        (global as any).CREDITCOIN_API_URL = creditcoinApiUrl(`ws://127.0.0.1:${wsPort}`);
    }

    if ((global as any).CREDITCOIN_HAS_SUDO === undefined) {
        (global as any).CREDITCOIN_HAS_SUDO = true;
    }
};

export default setup;
