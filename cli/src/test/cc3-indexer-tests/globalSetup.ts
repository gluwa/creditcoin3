import { KeyringPair, mnemonicGenerate } from '../../lib';
import { initKeyringPair } from '../../lib/account/keyring';

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

const setup = () => {
    (global as any).CREDITCOIN_API_URL = 'ws://127.0.0.1:9944';
    (global as any).GRAPHQL_URL = 'http://127.0.0.1:3000';

    (global as any).CREDITCOIN_CREATE_SIGNER = createSigner; // eslint-disable-line
};

export default setup;
