import { KeyringPair, Wallet, POINT_01_CTC, mnemonicGenerate } from '../lib';
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

const evmPrivateKey = (who: 'alice' | 'bob'): string => {
    // EVM private keys for development accounts are documented at
    // https://docs.moonbeam.network/builders/get-started/networks/moonbeam-dev/#pre-funded-development-accounts
    switch (who) {
        case 'alice': // Alith
            return '0x5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133';
        case 'bob': // Balthathar
            return '0x8075991ce870b93a8870eca0c0f91913d12f47948ca0fd25b49c6fa7cdbeee8b';
        default:
            throw new Error(`Unexpected value "${who}"`); // eslint-disable-line
    }
};

const setup = () => {
    process.env.NODE_ENV = 'test';

    if ((global as any).CREDITCOIN_CREATE_WALLET === undefined) {
        (global as any).CREDITCOIN_CREATE_WALLET = Wallet.createRandom; // eslint-disable-line
    }

    if ((global as any).CREDITCOIN_CREATE_SIGNER === undefined) {
        (global as any).CREDITCOIN_CREATE_SIGNER = createSigner; // eslint-disable-line
    }

    if ((global as any).CREDITCOIN_EVM_PRIVATE_KEY === undefined) {
        (global as any).CREDITCOIN_EVM_PRIVATE_KEY = evmPrivateKey; // eslint-disable-line
    }

    // WARNING: when setting global variables `undefined' means no value has been assigned
    // to this variable up to now so we fall-back to the defaults.
    // WARNING: don't change the comparison expression here b/c some variables are actually
    // configured to have a true or false value in different environments!

    if ((global as any).CREDITCOIN_API_URL === undefined) {
        const wsPort = process.env.CREDITCOIN_WS_PORT || '9944';
        (global as any).CREDITCOIN_API_URL = `ws://127.0.0.1:${wsPort}`;
    }

    if ((global as any).CREDITCOIN_MINIMUM_TXN_FEE === undefined) {
        (global as any).CREDITCOIN_MINIMUM_TXN_FEE = POINT_01_CTC;
    }

    if ((global as any).CREDITCOIN_HAS_SUDO === undefined) {
        (global as any).CREDITCOIN_HAS_SUDO = true;
    }

    if ((global as any).CREDITCOIN_USES_FAST_RUNTIME === undefined) {
        (global as any).CREDITCOIN_USES_FAST_RUNTIME = true;
    }

    if ((global as any).CREDITCOIN_HAS_EVM_TRACING === undefined) {
        (global as any).CREDITCOIN_HAS_EVM_TRACING = true;
    }
};

export default setup;

if (require.main === module) {
    console.log(evmPrivateKey('alice'));
}
