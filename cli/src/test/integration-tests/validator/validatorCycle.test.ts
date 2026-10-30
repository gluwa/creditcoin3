import { mnemonicValidate } from '@polkadot/util-crypto';
import { BN, newApi, ApiPromise, KeyringPair } from '../../../lib';
import { getBalance, printBalance } from '../../../lib/balance';
import { parseHexStringInternal } from '../../../lib/parsing';
import { getValidatorStatus } from '../../../lib/staking/validatorStatus';
import {
    ALICE_NODE_URL,
    BOB_NODE_URL,
    fundFromSudo,
    waitEras,
    initAliceKeyring,
    increaseValidatorCount,
    randomTestAccount,
    setUpProxy,
    tearDownProxy,
    CLIBuilder,
    fundedAccountPool,
} from '../helpers';
import { describeIf, isManualSealing, sealBlock, sleep } from '../../utils';
import { parseAmount } from '../../../commands/options';

async function waitForCompletedRewardEra(api: ApiPromise, validator: string, manualSealing: boolean): Promise<number> {
    const blockTime = api.consts.babe.expectedBlockTime.toNumber();
    const maxBlocks = api.consts.babe.epochDuration.toNumber() * api.consts.staking.sessionsPerEra.toNumber() * 3;
    const deadline = Date.now() + Math.max(60_000, (maxBlocks + 1) * blockTime);
    let rewardedEra: number | undefined;
    let activeEra = -1;
    let points = 0;

    for (let block = 0; block <= maxBlocks; block++) {
        // Read points and era completion from the same finalized state, including on BABE nodes.
        const state = await api.at(await api.rpc.chain.getFinalizedHead());
        activeEra = (await state.query.staking.activeEra()).unwrap().index.toNumber();
        const candidateEra = rewardedEra ?? activeEra;
        const rewardPoints = await state.query.staking.erasRewardPoints(candidateEra);
        points =
            [...rewardPoints.individual].find(([account]) => account.toString() === validator)?.[1].toNumber() ?? 0;
        if (points > 0) rewardedEra = candidateEra;

        if (rewardedEra !== undefined && activeEra > rewardedEra) {
            const payout = await state.query.staking.erasValidatorReward(rewardedEra);
            if (payout.isSome && !payout.unwrap().isZero()) {
                console.log(
                    `Finalized reward era ${rewardedEra}: validator points=${points}, era payout=${payout.toString()}`,
                );
                return rewardedEra;
            }
            throw new Error(
                `Completed era ${rewardedEra} has ${points} points for ${validator}, but payout is ${payout.toString()}`,
            );
        }

        if (block === maxBlocks || Date.now() >= deadline) break;
        if (manualSealing) await sealBlock(api);
        else await sleep(blockTime);
    }

    throw new Error(
        `No completed rewarded era for ${validator} within ${maxBlocks} blocks: ` +
            `activeEra=${activeEra}, rewardedEra=${rewardedEra ?? 'none'}, points=${points}`,
    );
}

describeIf(
    process.env.EXECUTE_VALIDATOR_CYCLE_TEST_SCENARIO === 'yes' &&
        (process.env.PROXY_ENABLED === undefined ||
            process.env.PROXY_ENABLED === 'no' ||
            (process.env.PROXY_ENABLED === 'yes' && process.env.PROXY_SECRET_VARIANT === 'valid-proxy')),
    'integration test: validator manual setup',
    () => {
        let api: ApiPromise;
        let proxy: any;
        let caller: any;
        let sudoSigner: KeyringPair;
        let accounts: ReturnType<typeof fundedAccountPool>;
        let CLI: any;
        let nonProxiedCli: any;

        beforeAll(async () => {
            ({ api } = await newApi(ALICE_NODE_URL));

            // Create a reference to sudo for funding accounts
            sudoSigner = initAliceKeyring();
            accounts = fundedAccountPool(api, sudoSigner);
            await increaseValidatorCount(api, sudoSigner);
        }, 60_000);

        beforeEach(async () => {
            const stashSecret = CLIBuilder({})('new').stdout.split('Seed phrase: ')[1];
            expect(mnemonicValidate(stashSecret)).toBe(true);
            console.log('Stash seed: ', stashSecret);

            caller = randomTestAccount(stashSecret);
            nonProxiedCli = CLIBuilder({ CC_SECRET: caller.secret });

            // Funding the stash account should make its balance equal to the amount funded
            const fundAmount = parseAmount('10000');
            await fundFromSudo(api, caller.address, fundAmount);
            const stashBalance = (await getBalance(caller.address, api)).transferable;
            expect(stashBalance.toString()).toBe(fundAmount.toString());

            // configure proxy
            proxy = await accounts.next();
            const wrongProxy = await accounts.next();
            CLI = await setUpProxy(api, nonProxiedCli, caller, proxy, wrongProxy);
        }, 120_000);

        afterEach(() => {
            tearDownProxy(nonProxiedCli, proxy);
        }, 30_000);

        afterAll(async () => {
            await api.disconnect();
        });

        it('full validator cycle', async () => {
            const manualSealing = await isManualSealing(api);
            // The manual-seal ticker produces blocks only on Alice; it must own the new
            // validator's keys as well as its original keys to award real authorship points.
            const authoringNodeUrl = manualSealing ? ALICE_NODE_URL : BOB_NODE_URL;

            // Bonding 1k ctc from stash
            const bondAmount = '1000';
            let result = nonProxiedCli(`bond --amount ${bondAmount}`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // wait 2 seconds for nodes to sync
            await sleep(2000);
            const stashStatus = await getValidatorStatus(caller.address, api);
            expect(stashStatus?.bonded).toBe(true);

            const stashBondedBalance = (await getBalance(caller.address, api)).bonded;
            expect(stashBondedBalance.toString()).toBe(parseAmount(bondAmount).toString());

            // Rotating session keys for the node should return a valid hex string
            const newKeys = parseHexStringInternal(
                nonProxiedCli(`rotate-keys --url ${authoringNodeUrl}`).stdout.split('New keys: ')[1],
            );

            // Setting session keys for the controller should
            // - make the validator (stash) next session keys equal to the new keys
            // - make the new keys appear as the node's session keys
            result = CLI(`set-keys --keys ${newKeys} --url ${BOB_NODE_URL}`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // wait 2 seconds for nodes to sync
            await sleep(2000);
            const validatorSessionKeys = await api.query.session.nextKeys(caller.address);
            expect(validatorSessionKeys.toHex()).toBe(newKeys);
            const authoringApi = (await newApi(authoringNodeUrl)).api;
            const nodeHasKeys = (await authoringApi.rpc.author.hasSessionKeys(newKeys)).isTrue;
            expect(nodeHasKeys).toBe(true);
            await authoringApi.disconnect();

            // Signaling intention to validate should make the validator (stash) appear as waiting
            result = CLI('validate --commission 1');
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            const stashStatusAfterValidating = await getValidatorStatus(caller.address, api);
            expect(stashStatusAfterValidating?.waiting).toBe(true);

            // wait for the next era, the validator should become elected & active.
            await waitEras(2, api);
            const stashStatusAfterEra = await getValidatorStatus(caller.address, api);
            expect(stashStatusAfterEra?.active).toBe(true);

            // Election alone does not imply rewards: prove authorship points, then wait
            // for that same era's finalized payout before exercising the CLI distribution.
            const startingEra = await waitForCompletedRewardEra(api, caller.address, manualSealing);

            // After distributing rewards, the validator staked balance should increase
            // (because it was set to staked)
            const balanceBeforeRewards = await getBalance(caller.address, api);
            console.log(balanceBeforeRewards.bonded.toString());

            result = CLI(`distribute-rewards --substrate-address ${caller.address} --era ${startingEra}`);
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // wait 2 seconds for nodes to sync
            await sleep(2000);
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

            // wait 2 seconds for nodes to sync
            await sleep(2000);
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
            await waitEras(unbondingPeriod, api);

            result = CLI('withdraw-unbonded');
            expect(result.exitCode).toEqual(0);
            expect(result.stdout).toContain('Transaction included at block');

            // wait 2 seconds for nodes to sync
            await sleep(2000);
            const balanceAfterWithdraw = await getBalance(caller.address, api);
            printBalance(balanceAfterWithdraw);
            const stashAmount = parseAmount('10000');
            expect(balanceAfterWithdraw.transferable.gte(stashAmount)).toBe(true);
        }, 2_000_000);
    },
);
