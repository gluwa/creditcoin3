import { initAliceKeyring, ALICE_NODE_URL, BOB_NODE_URL, randomFundedAccount, CLIBuilder } from './helpers';
import { newApi } from '../../lib';

describe('Proxy functionality', () => {
    it('Can list, add, and remove proxies for an account', async () => {
        // Setup
        const { api } = await newApi(ALICE_NODE_URL);

        // Create a reference to sudo for funding accounts
        const sudoSigner = initAliceKeyring();

        // Create and fund the test and proxy account
        const caller = await randomFundedAccount(api, sudoSigner);
        const proxy = await randomFundedAccount(api, sudoSigner);

        // Create a CLICmd instance with a properly configured environment
        // eslint-disable-next-line @typescript-eslint/naming-convention
        const CLI = CLIBuilder({ CC_SECRET: caller.secret, CC_PROXY_SECRET: proxy.secret });

        // Test #1. List proxies, should be empty
        const test1Res = CLI('proxy list');
        expect(test1Res.stdout).toContain('No proxies for address'); // Indicates no proxies have been set and 0 funds have been proxied

        // Test #2. Add the proxy with no errors
        const test2Res = CLI(`proxy add --proxy ${proxy.address} --type Staking --url ${BOB_NODE_URL}`);
        expect(test2Res.exitCode).toEqual(0);
        expect(test2Res.stdout).toContain('Transaction included at block');

        // Test #3. List the proxy and ensure it
        const test3Res = CLI(`proxy list --url ${BOB_NODE_URL}`);
        expect(test3Res.stdout).toContain(proxy.address); // The proxy address should be listed
        expect(test3Res.stdout).toContain('Staking'); // The type should be correctly listed as 'Staking'

        // Test #5. Successfully remove the proxy
        const test5Res = CLI(`proxy remove --proxy ${proxy.address} --url ${BOB_NODE_URL}`);
        expect(test5Res.exitCode).toEqual(0);
        expect(test2Res.stdout).toContain('Transaction included at block');

        // Test #6. List the proxies (should be empty )
        const test6Res = CLI(`proxy list --url ${BOB_NODE_URL}`);
        expect(test6Res.stdout).toContain('No proxies for address');

        await api.disconnect();
    }, 60000);

    it('Can successfully bond and unbond with a proxy account', async () => {
        // Setup
        const { api } = await newApi(ALICE_NODE_URL);

        // Create a reference to sudo for funding accounts
        const sudoSigner = initAliceKeyring();

        // Create and fund the test and proxy account
        const caller = await randomFundedAccount(api, sudoSigner);
        const proxy = await randomFundedAccount(api, sudoSigner);

        // Create a CLICmd instance with a properly configured environment
        // eslint-disable-next-line @typescript-eslint/naming-convention
        const CLI = CLIBuilder({ CC_SECRET: caller.secret, CC_PROXY_SECRET: proxy.secret });

        const setupRes = CLI(`proxy add --proxy ${proxy.address} --type Staking`);
        expect(setupRes.exitCode).toEqual(0);
        expect(setupRes.stdout).toContain('Transaction included at block');

        // Test #1. Successfully bond for the first time
        const test1Res = CLI(`bond --amount 1 --use-proxy ${caller.address}`);
        expect(test1Res.exitCode).toEqual(0);
        expect(test1Res.stdout).toContain('Transaction included at block');

        // Test #2. Attempt to bond extra without specifying the extra command
        // TODO This should fail but the signSendAndWatch function needs to be updated
        const test2Res = CLI(`bond --amount 1 --use-proxy ${caller.address}`);
        expect(test2Res.exitCode).toEqual(0);

        // Test #3. Successfully bond extra using the proxy
        const test3Res = CLI(`bond --amount 1 -x --use-proxy ${caller.address}`);
        expect(test3Res.exitCode).toEqual(0);
        expect(test3Res.stdout).toContain('Transaction included at block');

        // Test #4. Successfully unbond extra using the proxy
        const test4Res = CLI(`unbond --amount 1 --use-proxy ${caller.address}`);
        expect(test4Res.exitCode).toEqual(0);
        expect(test4Res.stdout).toContain('Transaction included at block');

        await api.disconnect();
    }, 60000);

    it('Can successfully validate and chill with a proxy account', async () => {
        // Setup
        const { api } = await newApi(ALICE_NODE_URL);

        // Create a reference to sudo for funding accounts
        const sudoSigner = initAliceKeyring();

        // Create and fund the test and proxy account
        const caller = await randomFundedAccount(api, sudoSigner);
        const proxy = await randomFundedAccount(api, sudoSigner);

        // Create a CLICmd instance with a properly configured environment
        // eslint-disable-next-line @typescript-eslint/naming-convention
        const CLI = CLIBuilder({ CC_SECRET: caller.secret, CC_PROXY_SECRET: proxy.secret });

        const setupRes = CLI(`proxy add --proxy ${proxy.address} --type Staking`);
        expect(setupRes.exitCode).toEqual(0);
        expect(setupRes.stdout).toContain('Transaction included at block');

        // Test #1. Successfully bond for the first time
        const test1Res = CLI(`bond --use-proxy ${caller.address} --amount 100`);
        expect(test1Res.exitCode).toEqual(0);
        expect(test1Res.stdout).toContain('Transaction included at block');

        // Test #2. Successfully bond for the first time
        const test2Res = CLI(`validate --use-proxy ${caller.address}`);
        expect(test2Res.exitCode).toEqual(0);
        expect(test2Res.stdout).toContain('Transaction included at block');

        // Test #3. Attempt to bond extra without specifying the extra command
        // TODO This should fail but the signSendAndWatch function needs to be updated
        const test3Res = CLI(`chill --use-proxy ${caller.address}`);
        expect(test3Res.exitCode).toEqual(0);
        expect(test3Res.stdout).toContain('Transaction included at block');

        await api.disconnect();
    }, 360_000);

    it('Can successfully send funds with a proxy', async () => {
        // Setup
        const { api } = await newApi(ALICE_NODE_URL);

        // Create a reference to sudo for funding accounts
        const sudoSigner = initAliceKeyring();

        // Create and fund the test and proxy account
        const caller = await randomFundedAccount(api, sudoSigner);
        const proxy = await randomFundedAccount(api, sudoSigner);

        // Create a CLICmd instance with a properly configured environment
        // eslint-disable-next-line @typescript-eslint/naming-convention
        const CLI = CLIBuilder({ CC_SECRET: caller.secret, CC_PROXY_SECRET: proxy.secret });

        const setupRes = CLI(`proxy add --proxy ${proxy.address} --type All`);
        expect(setupRes.exitCode).toEqual(0);
        expect(setupRes.stdout).toContain('Transaction included at block');

        // Test #1. Send money to Alice
        const test1Res = CLI(
            `send --amount 1 --use-proxy ${caller.address} --substrate-address 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY`,
        );
        expect(test1Res.exitCode).toEqual(0);
        expect(test1Res.stdout).toContain('Transaction included at block');

        await api.disconnect();
    }, 360_000);
});
