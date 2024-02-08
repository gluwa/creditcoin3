import { Command, OptionValues } from 'commander';
import { newApi, bond, MICROUNITS_PER_CTC, parseRewardDestination, BN, RewardDestination, ApiPromise } from '../../lib';
import { parseChoiceOrExit, inputOrDefault, parsePercentAsPerbillOrExit, parseBoolean } from '../../lib/parsing';
import { StakingPalletValidatorPrefs } from '../../lib/staking/validate';
import { TxStatus, requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../../lib/tx';
import { percentFromPerbill } from '../../lib/perbill';
import { CcKeyring, initKeyring, isProxy, delegateAddress } from '../../lib/account/keyring';
import { AccountBalance, getBalance, parseCTCString, printBalance, toCTCString } from '../../lib/balance';
import { promptContinue, promptContinueOrSkip, setInteractivity } from '../../lib/interactive';
import { amountOption, useProxyOption } from '../options';
import { isProxyFor } from '../../lib/proxy';

export function makeWizardCommand() {
    const cmd = new Command('wizard');
    cmd.description('Run the validator setup wizard. Only requires funded stash account.');
    cmd.option(
        '-r, --reward-destination [reward-destination]',
        'Specify reward destination account to use for new account',
    );
    cmd.option('--commission [commission]', 'Specify commission for validator');
    cmd.option('--blocked', 'Specify if validator is blocked for new nominations');
    cmd.addOption(useProxyOption);
    cmd.addOption(amountOption.default(new BN(0)));
    cmd.action(async (options: OptionValues) => {
        console.log('🧙 Running staking wizard...');

        const { amount, rewardDestination, commission, blocked, interactive, bondStep } = parseOptions(options);

        // Node settings
        const nodeUrl: string = options.url as string;

        // Create new API instance
        const { api } = await newApi(nodeUrl);

        // Generate keyring
        const keyring = await initKeyring(options);

        const address = delegateAddress(keyring);

        // Validate prefs
        const preferences: StakingPalletValidatorPrefs = {
            commission,
            blocked,
        };

        // State parameters being used
        console.log('Using the following parameters:');
        console.log(`💰 Stash account: ${address}`);

        if (isProxy(keyring)) {
            console.log(`⚠️ Using a proxy account! Stash should be bonded in advance!`);
            console.log(`🤐 Proxy account: ${keyring.pair.address}`);
        }
        if (bondStep && amount) {
            console.log(`🪙 Amount to bond: ${toCTCString(amount)}`);
        } else {
            console.log(`❌ Skipping bonding step (run with --amount [amount] flag to bond CTC)`);
        }
        console.log(`🎁 Reward destination: ${rewardDestination}`);
        console.log(`📡 Node URL: ${nodeUrl}`);
        console.log(`💸 Commission: ${percentFromPerbill(commission).toString()}`);
        console.log(`🔐 Blocked: ${blocked ? 'Yes' : 'No'}`);

        // Prompt continue
        await promptContinue(interactive);

        // Balance checks
        const grosslyEstimatedFee = parseCTCString('2');
        let balance;

        if (isProxy(keyring)) {
            await checkIfProxyIsValidOrExit(keyring.pair.address, address, api);
            const proxyBalance = await getBalance(keyring.pair.address, api);
            checkStashBalance(keyring.pair.address, proxyBalance, grosslyEstimatedFee);
            balance = proxyBalance;
        } else {
            const stashBalance = await getBalance(address, api);
            const amountWithFee = amount.add(grosslyEstimatedFee);
            checkStashBalance(address, stashBalance, amountWithFee);
            balance = stashBalance;
        }

        // Bond CTC
        if (bondStep) {
            await bondRoutine(keyring, address, balance, amount, rewardDestination, api, interactive);
        }

        // Rotate keys
        console.log('Generating new session keys on node...');
        const newKeys = (await api.rpc.author.rotateKeys()).toString();
        console.log('New node session keys:', newKeys);

        // Set keys
        console.log('Creating setKeys transaction...');
        const setKeysTx = api.tx.session.setKeys(newKeys, '');

        // Validate
        console.log('Creating validate transaction...');
        const validateTx = api.tx.staking.validate(preferences);

        // Send transactions
        console.log('Sending setKeys and validate transactions...');
        const txs = [setKeysTx, validateTx];

        const batchTx = api.tx.utility.batchAll(txs);
        await requireKeyringHasSufficientFunds(batchTx, keyring, api);

        const batchResult = await signSendAndWatchCcKeyring(batchTx, api, keyring);

        console.log(batchResult.info);

        if (batchResult.status === TxStatus.ok) {
            console.log('🧙 Validator wizard completed successfully!');
            console.log('Your validator should appear on the waiting queue.');
        }

        process.exit(0);
    });
    return cmd;
}

function checkStashBalance(address: string, balance: AccountBalance, amount: BN) {
    if (balance.transferable.lt(amount)) {
        console.log(`Account does not have enough funds, it requires ${toCTCString(amount)}`);
        printBalance(balance);
        console.log(`Please send funds to address ${address} and try again.`);
        process.exit(1);
    }
}

function checkIfAlreadyBonded(balance: AccountBalance) {
    if (balance.bonded.gt(new BN(0))) {
        return true;
    } else {
        return false;
    }
}

function parseOptions(options: OptionValues) {
    const interactive = setInteractivity(options);
    const amount = options.amount as BN;
    let bondStep = true;

    if (amount.gt(new BN(0))) {
        if (BigInt(amount.toString()) < BigInt(MICROUNITS_PER_CTC)) {
            console.log('Failed to setup wizard: Bond amount must be at least 1 CTC');
            process.exit(1);
        }
    } else {
        bondStep = false;
    }

    const rewardDestination = parseRewardDestination(
        parseChoiceOrExit(inputOrDefault(options.rewardDestination, 'Staked'), ['Staked', 'Stash']),
    );

    const commission = parsePercentAsPerbillOrExit(inputOrDefault(options.commission, '0'));

    const blocked = parseBoolean(options.blocked);

    const proxy = options.proxy;
    const proxeeAddress = options.address;

    return { amount, rewardDestination, commission, blocked, interactive, proxy, proxeeAddress, bondStep };
}

async function bondRoutine(
    keyring: CcKeyring,
    address: string,
    stashBalance: AccountBalance,
    amount: BN,
    rewardDestination: RewardDestination,
    api: ApiPromise,
    interactive: boolean,
) {
    // proxies and delegates are 'bonded' by default so if we are using one its always a bond extra extrinsic
    const bondExtra: boolean = checkIfAlreadyBonded(stashBalance);

    if (bondExtra) {
        console.log('⚠️  Warning: Stash account already bonded. This will increase the amount bonded.');
        if (isProxy(keyring)) {
            console.log('You do not need to bond extra funds if using a proxy');
        }
        if (await promptContinueOrSkip(`Continue or skip bonding extra funds?`, interactive)) {
            checkStashBalance(address, stashBalance, amount);
            // Bond extra
            console.log('Sending bond transaction...');
            const bondTxResult = await bond(keyring, amount, rewardDestination, api, bondExtra);
            console.log(bondTxResult.info);
            if (bondTxResult.status === TxStatus.failed) {
                console.log('Bond transaction failed. Exiting.');
                process.exit(1);
            }
        }
    } else {
        // Bond
        console.log('Sending bond transaction...');
        const bondTxResult = await bond(keyring, amount, rewardDestination, api, bondExtra);
        console.log(bondTxResult.info);
        if (bondTxResult.status === TxStatus.failed) {
            console.log('Bond transaction failed. Exiting.');
            process.exit(1);
        }
    }
}

async function checkIfProxyIsValidOrExit(proxy: string, proxee: string, api: ApiPromise) {
    const result = await isProxyFor(proxy, proxee, api);
    if (!result) {
        console.log(`Proxy ${proxy} is not valid for ${proxee}`);
        process.exit(1);
    }
}
