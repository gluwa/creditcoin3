import { mnemonicValidate } from '@polkadot/util-crypto';
import { commandSync } from 'execa';
import { newApi } from '../../api';
import { BN } from '../../lib';
import { getBalance, printBalance } from '../../lib/balance';
import {
    parseAddressInternal,
    parseAmountInternal,
    parseHexStringInternal,
} from '../../lib/parsing';
import { getValidatorStatus } from '../../lib/staking/validatorStatus';
import { signSendAndWatch } from '../../lib/tx';
import {
    BOB_NODE_URL,
    ALICE_NODE_URL,
    fundFromSudo,
    waitEras,
    initAlithKeyring,
} from './helpers';

describe('integration test: validator manual setup', () => {
    it('full validator cycle', async () => {
        // Bob's node is used for checking its configuration as a validator
        // and for sending extrinsics using the CLI
        const bobApi = (await newApi(BOB_NODE_URL)).api;

        // While CLI commands always send extrinsics through Bob's node,
        // sudo calls and state checks both use Alice's node
        const aliceApi = (await newApi(ALICE_NODE_URL)).api;

        const stashSecret = // Creating two accounts using `new` should return two valid mnemonic seeds
            commandSync('node dist/index.js new').stdout.split(
                'Seed phrase: '
            )[1];

        expect(mnemonicValidate(stashSecret)).toBe(true);

        console.log('Stash seed: ', stashSecret);

        // Getting the addresses using `show-address` should return two valid addresses
        const stashAddress = parseAddressInternal(
            commandSync(`node dist/index.js show-address`, {
                env: {
                    CC_SECRET: stashSecret,
                },
            }).stdout.split('Account address: ')[1]
        );

        // Funding the stash account should make its balance equal to the amount funded
        const fundAmount = parseAmountInternal('10000');

        const fundTx = await fundFromSudo(stashAddress, fundAmount);
        await signSendAndWatch(fundTx, aliceApi, initAlithKeyring());
        const stashBalance = (await getBalance(stashAddress, aliceApi))
            .transferable;
        expect(stashBalance.toString()).toBe(fundAmount.toString());

        // Bonding 1k ctc from stash and setting the controller should
        // - make the stash bonded balance equal to 1k ctc
        // - make the stash's controller be the controller address
        // - make controller's stash be the stash address
        const bondAmount = '1000';
        commandSync(
            `node dist/index.js bond --amount ${bondAmount} --url ${BOB_NODE_URL}`,
            {
                env: {
                    CC_STASH_SECRET: stashSecret,
                },
            }
        );
        // wait 5 seconds for nodes to sync
        await new Promise((resolve) => setTimeout(resolve, 5000));
        const stashStatus = await getValidatorStatus(stashAddress, aliceApi);
        expect(stashStatus.bonded).toBe(true);

        const stashBondedBalance = (await getBalance(stashAddress, aliceApi))
            .bonded;
        expect(stashBondedBalance.toString()).toBe(
            parseAmountInternal(bondAmount).toString()
        );

        // Rotating session keys for the node should return a valid hex string
        const newKeys = parseHexStringInternal(
            commandSync(
                `node dist/index.js rotate-keys --url ${BOB_NODE_URL}`
            ).stdout.split('New keys: ')[1]
        );

        // Setting session keys for the controller should
        // - make the validator (stash) next session keys equal to the new keys
        // - make the new keys appear as the node's session keys
        commandSync(
            `node dist/index.js set-keys --keys ${newKeys} --url ${BOB_NODE_URL}`,
            {
                env: {
                    CC_STASH_SECRET: stashSecret,
                },
            }
        );
        // wait 5 seconds for nodes to sync
        await new Promise((resolve) => setTimeout(resolve, 5000));
        const validatorSessionKeys =
            await aliceApi.query.session.nextKeys(stashAddress);
        expect(validatorSessionKeys.toHex()).toBe(newKeys);
        const nodeHasKeys = (await bobApi.rpc.author.hasSessionKeys(newKeys))
            .isTrue;
        expect(nodeHasKeys).toBe(true);

        // Signaling intention to validate should make the validator (stash) appear as waiting
        commandSync(
            `node dist/index.js validate --commission 1 --url ${BOB_NODE_URL}`,
            {
                env: {
                    CC_STASH_SECRET: stashSecret,
                },
            }
        );

        const stashStatusAfterValidating = await getValidatorStatus(
            stashAddress,
            bobApi
        );
        expect(stashStatusAfterValidating.waiting).toBe(true);

        // After increasing the validator count, (forcing an era- currently not) and waiting for the next era,
        // the validator should become elected & active.
        const increaseValidatorCountTx =
            aliceApi.tx.staking.setValidatorCount(2);
        const increaseValidatorCountSudoTx = aliceApi.tx.sudo.sudo(
            increaseValidatorCountTx
        );
        await signSendAndWatch(
            increaseValidatorCountSudoTx,
            aliceApi,
            initAlithKeyring()
        );
        const validatorCount = (
            await aliceApi.query.staking.validatorCount()
        ).toNumber();
        expect(validatorCount).toBe(2);
        await waitEras(2, aliceApi);
        const stashStatusAfterEra = await getValidatorStatus(
            stashAddress,
            bobApi
        );
        expect(stashStatusAfterEra.active).toBe(true);

        // After waiting for another era, the validator should have accumulated era rewards to distribute
        const startingEra = (
            await aliceApi.derive.session.info()
        ).activeEra.toNumber();
        console.log('Starting era: ', startingEra);
        await waitEras(1, aliceApi);

        // After distributing rewards, the validator staked balance should increase
        // (because it was set to staked)
        const balanceBeforeRewards = await getBalance(stashAddress, aliceApi);
        console.log(balanceBeforeRewards.bonded.toString());

        commandSync(
            `node dist/index.js distribute-rewards --url ${BOB_NODE_URL} --validator-id ${stashAddress} --era ${startingEra}`,
            {
                env: {
                    CC_SECRET: stashSecret,
                },
            }
        );

        // wait 5 seconds for nodes to sync
        await new Promise((resolve) => setTimeout(resolve, 5000));
        const balanceAfterRewards = await getBalance(stashAddress, aliceApi);
        console.log(balanceAfterRewards.bonded.toString());
        const balanceIncreased = balanceAfterRewards.bonded.gt(
            balanceBeforeRewards.bonded
        );
        expect(balanceIncreased).toBe(true);

        // After executing the chill commmand, the validator should no longer be active nor waiting
        commandSync(`node dist/index.js chill --url ${BOB_NODE_URL}`, {
            env: {
                CC_STASH_SECRET: stashSecret,
            },
        });
        // wait 5 seconds for nodes to sync
        await waitEras(2, aliceApi);
        const stashStatusAfterChill = await getValidatorStatus(
            stashAddress,
            bobApi
        );
        expect(stashStatusAfterChill.active).toBe(false);
        expect(stashStatusAfterChill.waiting).toBe(false);

        // After unbonding, the validator should no longer be bonded
        commandSync(
            // Unbonding defaults to max if it exceeds the bonded amount
            `node dist/index.js unbond --url ${BOB_NODE_URL} -a 100000`,
            {
                env: {
                    CC_STASH_SECRET: stashSecret,
                },
            }
        );
        // wait 5 seconds for nodes to sync
        await new Promise((resolve) => setTimeout(resolve, 5000));
        const balanceAfterUnbonding = await getBalance(stashAddress, aliceApi);
        const isUnbonding = balanceAfterUnbonding.unbonding.gt(new BN(0));
        printBalance(balanceAfterRewards);
        printBalance(balanceAfterUnbonding);
        const isUnbondingAll = balanceAfterUnbonding.unbonding.eq(
            balanceAfterRewards.bonded
        );
        expect(isUnbonding).toBe(true);
        expect(isUnbondingAll).toBe(true);

        // After unbonding and waiting for the unbonding period, the validator should be able to withdraw
        // the unbonded amount and end up with more funds than the initial funding
        const unbondingPeriod =
            aliceApi.consts.staking.bondingDuration.toNumber();
        console.log('Unbonding period: ', unbondingPeriod);
        await waitEras(unbondingPeriod + 1, aliceApi, true);

        commandSync(
            `node dist/index.js withdraw-unbonded --url ${BOB_NODE_URL}`,
            {
                env: {
                    CC_STASH_SECRET: stashSecret,
                },
            }
        );

        // wait 5 seconds for nodes to sync
        await new Promise((resolve) => setTimeout(resolve, 5000));
        const balanceAfterWithdraw = await getBalance(stashAddress, aliceApi);
        printBalance(balanceAfterWithdraw);
        const stashAmount = fundAmount;
        expect(balanceAfterWithdraw.transferable.gte(stashAmount)).toBe(true);

        await aliceApi.disconnect();
        await bobApi.disconnect();
    }, 2000000);
});
