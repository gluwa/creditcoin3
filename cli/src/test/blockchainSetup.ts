import { KeyringPair, Wallet, POINT_01_CTC, mnemonicGenerate } from '../lib';
import { initKeyringPair } from '../lib/account/keyring';

const createSigner = (who: 'lender' | 'borrower' | 'random' | 'sudo'): KeyringPair => {
    switch (who) {
        case 'lender':
            return initKeyringPair('//Alice');
        case 'borrower':
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
    process.env.NODE_ENV = 'test';

    if ((global as any).CREDITCOIN_CREATE_WALLET === undefined) {
        (global as any).CREDITCOIN_CREATE_WALLET = Wallet.createRandom; // eslint-disable-line
    }

    if ((global as any).CREDITCOIN_CREATE_SIGNER === undefined) {
        (global as any).CREDITCOIN_CREATE_SIGNER = createSigner; // eslint-disable-line
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
};

export default setup;
