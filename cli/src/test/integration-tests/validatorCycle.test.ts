import { mnemonicValidate } from '@polkadot/util-crypto';
import { BN, newApi, ApiPromise, KeyringPair } from '../../lib';
import { getBalance, printBalance } from '../../lib/balance';
import { parseHexStringInternal } from '../../lib/parsing';
import { getValidatorStatus } from '../../lib/staking/validatorStatus';
import { signSendAndWatch } from '../../lib/tx';
import {
    ALICE_NODE_URL,
    BOB_NODE_URL,
    fundFromSudo,
    waitEras,
    initAliceKeyring,
    increaseValidatorCount,
    randomFundedAccount,
    randomTestAccount,
    setUpProxy,
    tearDownProxy,
    CLIBuilder,
} from './helpers';
import { describeIf } from '../utils';
import { parseAmount } from '../../commands/options';

describeIf(
    process.env.PROXY_ENABLED === undefined ||
        process.env.PROXY_ENABLED === 'no' ||
        (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy'),
    'integration test: validator manual setup',
    () => {
        let api: ApiPromise;
        let proxy: any;
        let caller: any;
        let sudoSigner: KeyringPair;
        let CLI: any;
        let nonProxiedCli: any;

        beforeAll(async () => {
            ({ api } = await newApi(ALICE_NODE_URL));

            // Create a reference to sudo for funding accounts
            sudoSigner = initAliceKeyring();
        });

        beforeEach(async () => {
            const stashSecret = CLIBuilder({})('new').stdout.split('Seed phrase: ')[1];
            expect(mnemonicValidate(stashSecret)).toBe(true);
            console.log('Stash seed: ', stashSecret);

            caller = randomTestAccount(stashSecret);
            nonProxiedCli = CLIBuilder({ CC_SECRET: caller.secret });

            // Funding the stash account should make its balance equal to the amount funded
            const fundAmount = parseAmount('10000');
            const fundTx = await fundFromSudo(caller.address, fundAmount);
            await signSendAndWatch(fundTx, api, initAliceKeyring());
            const stashBalance = (await getBalance(caller.address, api)).transferable;
            expect(stashBalance.toString()).toBe(fundAmount.toString());

            // configure proxy
            proxy = await randomFundedAccount(api, sudoSigner);
            const wrongProxy = await randomFundedAccount(api, sudoSigner);
            CLI = await setUpProxy(nonProxiedCli, caller, proxy, wrongProxy);
        }, 120_000);

        afterEach(() => {
            tearDownProxy(nonProxiedCli, proxy);
        }, 30_000);

        afterAll(async () => {
            await api.disconnect();
        });

        it('full validator cycle', async () => {
            // Bonding 1k ctc from stash
            const bondAmount = '1000';
            let result = nonProxiedCli(`bond --amount ${bondAmount}`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // wait 5 seconds for nodes to sync
            await new Promise((resolve) => setTimeout(resolve, 5000));
            const stashStatus = await getValidatorStatus(caller.address, api);
            expect(stashStatus?.bonded).toBe(true);

            const stashBondedBalance = (await getBalance(caller.address, api)).bonded;
            expect(stashBondedBalance.toString()).toBe(parseAmount(bondAmount).toString());

            // Rotating session keys for the node should return a valid hex string
            const newKeys = parseHexStringInternal(
                nonProxiedCli(`rotate-keys --url ${BOB_NODE_URL}`).stdout.split('New keys: ')[1],
            );

            // Setting session keys for the controller should
            // - make the validator (stash) next session keys equal to the new keys
            // - make the new keys appear as the node's session keys
            result = CLI(`set-keys --keys ${newKeys} --url ${BOB_NODE_URL}`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // wait 5 seconds for nodes to sync
            await new Promise((resolve) => setTimeout(resolve, 5000));
            const validatorSessionKeys = await api.query.session.nextKeys(caller.address);
            expect(validatorSessionKeys.toHex()).toBe(newKeys);
            const bobApi = (await newApi(BOB_NODE_URL)).api;
            const nodeHasKeys = (await bobApi.rpc.author.hasSessionKeys(newKeys)).isTrue;
            expect(nodeHasKeys).toBe(true);
            await bobApi.disconnect();

            // Signaling intention to validate should make the validator (stash) appear as waiting
            result = CLI('validate --commission 1');
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            const stashStatusAfterValidating = await getValidatorStatus(caller.address, api);
            expect(stashStatusAfterValidating?.waiting).toBe(true);

            // After increasing the validator count, (forcing an era- currently not) and waiting for the next era,
            // the validator should become elected & active.
            await increaseValidatorCount(api, initAliceKeyring(), 2);
            await waitEras(2, api);
            const stashStatusAfterEra = await getValidatorStatus(caller.address, api);
            expect(stashStatusAfterEra?.active).toBe(true);

            // After waiting for another era, the validator should have accumulated era rewards to distribute
            const startingEra = (await api.derive.session.info()).activeEra.toNumber();
            console.log('Starting era: ', startingEra);
            await waitEras(1, api);

            // After distributing rewards, the validator staked balance should increase
            // (because it was set to staked)
            const balanceBeforeRewards = await getBalance(caller.address, api);
            console.log(balanceBeforeRewards.bonded.toString());

            result = CLI(`distribute-rewards --substrate-address ${caller.address} --era ${startingEra}`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // wait 5 seconds for nodes to sync
            await new Promise((resolve) => setTimeout(resolve, 5000));
            const balanceAfterRewards = await getBalance(caller.address, api);
            console.log(balanceAfterRewards.bonded.toString());
            const balanceIncreased = balanceAfterRewards.bonded.gt(balanceBeforeRewards.bonded);
            expect(balanceIncreased).toBe(true);

            // After executing the chill commmand, the validator should no longer be active nor waiting
            result = CLI('chill');
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // wait 5 seconds for nodes to sync
            await waitEras(2, api);
            const stashStatusAfterChill = await getValidatorStatus(caller.address, api);
            expect(stashStatusAfterChill?.active).toBe(false);
            expect(stashStatusAfterChill?.waiting).toBe(false);

            // After unbonding, the validator should no longer be bonded
            result = CLI(
                // Unbonding defaults to max if it exceeds the bonded amount
                'unbond --amount 100000',
            );
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // wait 5 seconds for nodes to sync
            await new Promise((resolve) => setTimeout(resolve, 5000));
            const balanceAfterUnbonding = await getBalance(caller.address, api);
            const isUnbonding = balanceAfterUnbonding.unbonding.gt(new BN(0));
            printBalance(balanceAfterRewards);
            printBalance(balanceAfterUnbonding);
            const isUnbondingAll = balanceAfterUnbonding.unbonding.eq(balanceAfterRewards.bonded);
            expect(isUnbonding).toBe(true);
            expect(isUnbondingAll).toBe(true);

            // After unbonding and waiting for the unbonding period, the validator should be able to withdraw
            // the unbonded amount and end up with more funds than the initial funding
            const unbondingPeriod: number = api.consts.staking.bondingDuration.toNumber();
            console.log('Unbonding period: ', unbondingPeriod);
            await waitEras(unbondingPeriod + 1, api, true);

            result = CLI('withdraw-unbonded');
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // wait 5 seconds for nodes to sync
            await new Promise((resolve) => setTimeout(resolve, 5000));
            const balanceAfterWithdraw = await getBalance(caller.address, api);
            printBalance(balanceAfterWithdraw);
            const stashAmount = parseAmount('10000');
            expect(balanceAfterWithdraw.transferable.gte(stashAmount)).toBe(true);
        }, 2_000_000);
    },
);
