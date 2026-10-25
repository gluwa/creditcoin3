import { KeyringPair } from '../lib';
import { initKeyringPair } from '../lib/account/keyring';
import { default as globalSetup, creditcoinApiUrl } from './blockchainSetup';

const createSigner = (who: 'alice' | 'bob' | 'random' | 'sudo'): KeyringPair => {
    switch (who) {
        case 'alice':
            const aliceSeed = process.env.ALICE_SEED;
            if (aliceSeed === undefined) {
                throw new Error('ALICE_SEED environment variable is required');
            }
            return initKeyringPair(aliceSeed!); // eslint-disable-line
        case 'bob':
            const bobSeed = process.env.BOB_SEED;
            if (bobSeed === undefined) {
                throw new Error('BOB_SEED environment variable is required');
            }
            return initKeyringPair(bobSeed!); // eslint-disable-line
        case 'random':
            // throw an error for now b/c we want to be careful sending funds during testing
            // which we may potentially not be able to retrieve!
            throw new Error('Reconsider generating random accounts for testing in this environent');
        case 'sudo':
            const sudoSeed = process.env.SUDO_SEED;
            if (sudoSeed === undefined) {
                throw new Error('SUDO_SEED environment variable is required');
            }
            return initKeyringPair(sudoSeed!); // eslint-disable-line
        default:
            throw new Error(`Unexpected value "${who}"`); // eslint-disable-line
    }
};

const evmPrivateKey = (who: 'alice' | 'bob'): string => {
    // WARNING: EVM Alice != Substrate Alice
    // WARNING: EVM Alice != Substrate Alice's Associated EVM address
    // Explanation: the accounts used on the Substrate and EVM side are individual
    // and not related to each other in any way. We refer to them using the same
    // handles in order to make it more obvious which actor is performing specific
    // transactions as part of the test suite! You can think about these accounts
    // as if Alice & Bob control multiple accounts in their wallets!
    switch (who) {
        case 'alice':
            const alicePk = process.env.ALICE_EVM_PK;
            if (alicePk === undefined) {
                throw new Error('ALICE_EVM_PK environment variable is required');
            }
            return alicePk;
        case 'bob':
            const bobPk = process.env.BOB_EVM_PK;
            if (bobPk === undefined) {
                throw new Error('BOB_EVM_PK environment variable is required');
            }
            return bobPk;
        default:
            throw new Error(`Unexpected value "${who}"`); // eslint-disable-line
    }
};

const setup = () => {
    (global as any).CREDITCOIN_EXPECTED_EPOCH_DURATION = 2880;
    (global as any).CREDITCOIN_EXPECTED_BLOCK_TIME = 15000;
    (global as any).CREDITCOIN_HAS_EVM_TRACING = false;

    (global as any).CREDITCOIN_CREATE_SIGNER = createSigner;
    (global as any).CREDITCOIN_EVM_PRIVATE_KEY = evmPrivateKey;
    (global as any).CREDITCOIN_API_URL = creditcoinApiUrl('wss://rpc.cc3-testnet.creditcoin.network/ws');

    globalSetup();
};

export default setup;

if (require.main === module) {
    console.log(evmPrivateKey('alice'));
}
