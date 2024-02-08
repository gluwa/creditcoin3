import { commandSync } from 'execa';
import { parseAmount } from '../../commands/options';
import { signSendAndWatch } from '../../lib/tx';
import {
    randomTestAccount,
    fundAddressesFromSudo,
    ALICE_NODE_URL,
    BOB_NODE_URL,
    initAliceKeyring,
    CLI_PATH,
} from './helpers';
import { getValidatorStatus } from '../../lib/staking/validatorStatus';
import { newApi, ApiPromise } from '../../lib';

describe('integration test: validator wizard setup', () => {
    let api: ApiPromise;

    beforeAll(async () => {
        ({ api } = await newApi(ALICE_NODE_URL));
    });

    afterAll(async () => {
        await api.disconnect();
    });

    it('new validator should appear as waiting after running', async () => {
        // Fund stash and controller
        const stash = randomTestAccount();

        const fundTx = await fundAddressesFromSudo([stash.address], parseAmount('10000'));
        await signSendAndWatch(fundTx, api, initAliceKeyring());

        // Run wizard setup with 1k ctc ang to pair with node Bob
        commandSync(`node ${CLI_PATH} wizard --amount 1000 --url ${BOB_NODE_URL}`, {
            env: {
                CC_SECRET: stash.secret,
            },
        });

        const validatorStatus = await getValidatorStatus(stash.address, api);

        expect(validatorStatus?.waiting).toBe(true);
        console.log('Validator waiting status is: ', validatorStatus?.waiting);
    }, 120000);

    it('new validator should appear as waiting after running wizard with a proxy', async () => {
        // Fund stash and proxy
        const stash = randomTestAccount();
        const proxy = randomTestAccount();

        const fundTx = await fundAddressesFromSudo([stash.address, proxy.address], parseAmount('10000'));
        await signSendAndWatch(fundTx, api, initAliceKeyring());

        // Add a staking proxy
        commandSync(`node ${CLI_PATH} proxy add --proxy ${proxy.address} --type Staking`, {
            env: {
                CC_SECRET: stash.secret,
            },
        });
        // Run wizard setup using the Proxy with 1k ctc to pair with node Bob
        commandSync(`node ${CLI_PATH} wizard --url ${BOB_NODE_URL} --use-proxy ${stash.address} --amount 1000`, {
            env: {
                CC_PROXY_SECRET: proxy.secret,
            },
        });

        const validatorStatus = await getValidatorStatus(stash.address, api);

        expect(validatorStatus?.bonded).toBe(true);
        expect(validatorStatus?.validating).toBe(true);
        expect(validatorStatus?.waiting).toBe(true);
        console.log('Validator waiting status is: ', validatorStatus?.waiting);
    }, 120000);
});
