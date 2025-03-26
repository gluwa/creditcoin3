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

const evmPrivateKey = (who: 'alice' | 'bob'): string => {
    // EVM private keys for development accounts are documented at
    // https://docs.moonbeam.network/builders/get-started/networks/moonbeam-dev/#pre-funded-development-accounts
    switch (who) {
        case 'alice': // Alith
            // 5Fghzk1AJt88PeFEzuRfXzbPchiBbsVGTTXcdx599VdZzkTA 0xf24FF3a9CF04c71Dbc94D0b566f7A27B94566cac
            return '0x5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133';
        case 'bob': // Balthathar
            return '0x8075991ce870b93a8870eca0c0f91913d12f47948ca0fd25b49c6fa7cdbeee8b';
        default:
            throw new Error(`Unexpected value "${who}"`); // eslint-disable-line
    }
};

const setup = () => {
    (global as any).ANVIL1_URL = 'http://127.0.0.1:8141';
    (global as any).CREDITCOIN_API_URL = 'ws://127.0.0.1:9944';
    (global as any).GRAPHQL_URL = 'http://127.0.0.1:3000';

    (global as any).CREDITCOIN_CREATE_SIGNER = createSigner; // eslint-disable-line
    (global as any).CREDITCOIN_EVM_PRIVATE_KEY = evmPrivateKey; // eslint-disable-line
};

export default setup;
