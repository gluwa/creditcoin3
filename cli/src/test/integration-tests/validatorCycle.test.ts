import { mnemonicValidate } from '@polkadot/util-crypto';
import { commandSync } from 'execa';
import { BN, newApi } from '../../lib';
import { getBalance, printBalance } from '../../lib/balance';
import { parseHexStringInternal } from '../../lib/parsing';
import { getValidatorStatus } from '../../lib/staking/validatorStatus';
import { signSendAndWatch } from '../../lib/tx';
import { BOB_NODE_URL, ALICE_NODE_URL, fundFromSudo, waitEras, initAliceKeyring, CLI_PATH } from './helpers';
import { isAddress } from 'ethers';
import { parseAmount, parseEVMAddress, parseSubstrateAddress } from '../../commands/options';

describe('integration test: validator manual setup', () => {
    it('full validator cycle', async () => {
        // Bob's node is used for checking its configuration as a validator
        // and for sending extrinsics using the CLI
        const bobApi = (await newApi(BOB_NODE_URL)).api;

        // While CLI commands always send extrinsics through Bob's node,
        // sudo calls and state checks both use Alice's node
        const aliceApi = (await newApi(ALICE_NODE_URL)).api;

        const stashSecret = // Creating two accounts using `new` should return two valid mnemonic seeds
            commandSync(`node ${CLI_PATH} new`).stdout.split('Seed phrase: ')[1];

        expect(mnemonicValidate(stashSecret)).toBe(true);

        console.log('Stash seed: ', stashSecret);

        // Getting both Substrate and EVM addresses using `show-address` should return two valid addresses
        const showAddressResult = commandSync(`node ${CLI_PATH} show-address`, {
            env: {
                CC_SECRET: stashSecret,
            },
        }).stdout;

        const substrateAddress = parseSubstrateAddress(
            showAddressResult
                .split(/\r?\n/)[0] // First line of the output
                .split('Account Substrate address: ')[1], // Substrate address
        );

        const evmAddress = parseEVMAddress(
            showAddressResult
                .split(/\r?\n/)[1] // Second line of the output
                .split('Associated EVM address: ')[1], // EVM address
        );

        expect(isAddress(evmAddress)).toBe(true);

        // Funding the stash account should make its balance equal to the amount funded
        const fundAmount = parseAmount('10000');

        const fundTx = await fundFromSudo(substrateAddress, fundAmount);
        await signSendAndWatch(fundTx, aliceApi, initAliceKeyring());
        const stashBalance = (await getBalance(substrateAddress, aliceApi)).transferable;
        expect(stashBalance.toString()).toBe(fundAmount.toString());

        // Bonding 1k ctc from stash
        const bondAmount = '1000';
        commandSync(`node ${CLI_PATH} bond --amount ${bondAmount} --url ${BOB_NODE_URL}`, {
            env: {
                CC_SECRET: stashSecret,
            },
        });
        // wait 5 seconds for nodes to sync
        await new Promise((resolve) => setTimeout(resolve, 5000));
        const stashStatus = await getValidatorStatus(substrateAddress, aliceApi);
        expect(stashStatus.bonded).toBe(true);

        const stashBondedBalance = (await getBalance(substrateAddress, aliceApi)).bonded;
        expect(stashBondedBalance.toString()).toBe(parseAmount(bondAmount).toString());

        // Rotating session keys for the node should return a valid hex string
        const newKeys = parseHexStringInternal(
            commandSync(`node ${CLI_PATH} rotate-keys --url ${BOB_NODE_URL}`).stdout.split('New keys: ')[1],
        );

        // Setting session keys for the controller should
        // - make the validator (stash) next session keys equal to the new keys
        // - make the new keys appear as the node's session keys
        commandSync(`node ${CLI_PATH} set-keys --keys ${newKeys} --url ${BOB_NODE_URL}`, {
            env: {
                CC_SECRET: stashSecret,
            },
        });
        // wait 5 seconds for nodes to sync
        await new Promise((resolve) => setTimeout(resolve, 5000));
        const validatorSessionKeys = await aliceApi.query.session.nextKeys(substrateAddress);
        expect(validatorSessionKeys.toHex()).toBe(newKeys);
        const nodeHasKeys = (await bobApi.rpc.author.hasSessionKeys(newKeys)).isTrue;
        expect(nodeHasKeys).toBe(true);

        // Signaling intention to validate should make the validator (stash) appear as waiting
        commandSync(`node ${CLI_PATH} validate --commission 1 --url ${BOB_NODE_URL}`, {
            env: {
                CC_SECRET: stashSecret,
            },
        });

        const stashStatusAfterValidating = await getValidatorStatus(substrateAddress, bobApi);
        expect(stashStatusAfterValidating.waiting).toBe(true);

        // After increasing the validator count, (forcing an era- currently not) and waiting for the next era,
        // the validator should become elected & active.
        const increaseValidatorCountTx = aliceApi.tx.staking.setValidatorCount(2);
        const increaseValidatorCountSudoTx = aliceApi.tx.sudo.sudo(increaseValidatorCountTx);
        await signSendAndWatch(increaseValidatorCountSudoTx, aliceApi, initAliceKeyring());
        const validatorCount = (await aliceApi.query.staking.validatorCount()).toNumber();
        expect(validatorCount).toBe(2);
        await waitEras(2, aliceApi);
        const stashStatusAfterEra = await getValidatorStatus(substrateAddress, bobApi);
        expect(stashStatusAfterEra.active).toBe(true);

        // After waiting for another era, the validator should have accumulated era rewards to distribute
        const startingEra = (await aliceApi.derive.session.info()).activeEra.toNumber();
        console.log('Starting era: ', startingEra);
        await waitEras(1, aliceApi);

        // After distributing rewards, the validator staked balance should increase
        // (because it was set to staked)
        const balanceBeforeRewards = await getBalance(substrateAddress, aliceApi);
        console.log(balanceBeforeRewards.bonded.toString());

        commandSync(
            `node ${CLI_PATH} distribute-rewards --url ${BOB_NODE_URL} --substrate-address ${substrateAddress} --era ${startingEra}`,
            {
                env: {
                    CC_SECRET: stashSecret,
                },
            },
        );

        // wait 5 seconds for nodes to sync
        await new Promise((resolve) => setTimeout(resolve, 5000));
        const balanceAfterRewards = await getBalance(substrateAddress, aliceApi);
        console.log(balanceAfterRewards.bonded.toString());
        const balanceIncreased = balanceAfterRewards.bonded.gt(balanceBeforeRewards.bonded);
        expect(balanceIncreased).toBe(true);

        // After executing the chill commmand, the validator should no longer be active nor waiting
        commandSync(`node ${CLI_PATH} chill --url ${BOB_NODE_URL}`, {
            env: {
                CC_SECRET: stashSecret,
            },
        });
        // wait 5 seconds for nodes to sync
        await waitEras(2, aliceApi);
        const stashStatusAfterChill = await getValidatorStatus(substrateAddress, bobApi);
        expect(stashStatusAfterChill.active).toBe(false);
        expect(stashStatusAfterChill.waiting).toBe(false);

        // After unbonding, the validator should no longer be bonded
        commandSync(
            // Unbonding defaults to max if it exceeds the bonded amount
            `node ${CLI_PATH} unbond --url ${BOB_NODE_URL} --amount 100000`,
            {
                env: {
                    CC_SECRET: stashSecret,
                },
            },
        );
        // wait 5 seconds for nodes to sync
        await new Promise((resolve) => setTimeout(resolve, 5000));
        const balanceAfterUnbonding = await getBalance(substrateAddress, aliceApi);
        const isUnbonding = balanceAfterUnbonding.unbonding.gt(new BN(0));
        printBalance(balanceAfterRewards);
        printBalance(balanceAfterUnbonding);
        const isUnbondingAll = balanceAfterUnbonding.unbonding.eq(balanceAfterRewards.bonded);
        expect(isUnbonding).toBe(true);
        expect(isUnbondingAll).toBe(true);

        // After unbonding and waiting for the unbonding period, the validator should be able to withdraw
        // the unbonded amount and end up with more funds than the initial funding
        const unbondingPeriod = aliceApi.consts.staking.bondingDuration.toNumber();
        console.log('Unbonding period: ', unbondingPeriod);
        await waitEras(unbondingPeriod + 1, aliceApi, true);

        commandSync(`node ${CLI_PATH} withdraw-unbonded --url ${BOB_NODE_URL}`, {
            env: {
                CC_SECRET: stashSecret,
            },
        });

        // wait 5 seconds for nodes to sync
        await new Promise((resolve) => setTimeout(resolve, 5000));
        const balanceAfterWithdraw = await getBalance(substrateAddress, aliceApi);
        printBalance(balanceAfterWithdraw);
        const stashAmount = fundAmount;
        expect(balanceAfterWithdraw.transferable.gte(stashAmount)).toBe(true);

        await aliceApi.disconnect();
        await bobApi.disconnect();
    }, 2000000);
});
