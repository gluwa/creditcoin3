import { Command, OptionValues } from 'commander';
import { getValidatorStatus, newApi, requireStatus } from '../../lib';
import { requireEnoughFundsToSend, signSendAndWatch } from '../../lib/tx';
import { initCallerKeyring, initProxyKeyring } from '../../lib/account/keyring';

export function makeWithdrawUnbondedCommand() {
    const cmd = new Command('withdraw-unbonded');
    cmd.description('Withdraw unbonded funds from a stash account');
    cmd.option('-p, --proxy', 'Whether to use a proxy account');
    cmd.option('-a, --address [address]', 'The address of the proxied account (use only with -p, --proxy');
    cmd.action(withdrawUnbondedAction);
    return cmd;
}

async function withdrawUnbondedAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const keyring = await initCallerKeyring(options);
    const proxy = await initProxyKeyring(options);
    const addr = proxy ? options.address : keyring.address;

    const status = await getValidatorStatus(addr, api);
    requireStatus(status, 'canWithdraw', 'Cannot perform action, there are no unlocked funds to withdraw');

    const slashingSpans = await api.query.staking.slashingSpans(keyring?.address as string);
    const slashingSpansCount = slashingSpans.isSome ? slashingSpans.unwrap().lastNonzeroSlash : 0;

    let withdrawUnbondTx = api.tx.staking.withdrawUnbonded(slashingSpansCount);
    let callerKeyring = keyring;
    let callerAddress = keyring?.address;

    if (options.proxy) {
        if (!options.address) {
            console.log("ERROR: Address not supplied, provide with '--address <address>'");
            process.exit(1);
        }
        if (!proxy) {
            console.log('ERROR: proxy keyring not provided through $PROXY_SECRET or interactive prompt');
            process.exit(1);
        }

        withdrawUnbondTx = api.tx.proxy.proxy(options.address, null, withdrawUnbondTx);
        callerAddress = proxy.address;
        callerKeyring = proxy;
    }

    if (!callerAddress) {
        console.log('ERROR: caller address not initialized');
        process.exit(1);
    }
    if (!callerKeyring) {
        console.log('ERROR: caller keyring not initialized');
        process.exit(1);
    }

    await requireEnoughFundsToSend(withdrawUnbondTx, callerAddress, api);
    const result = await signSendAndWatch(withdrawUnbondTx, api, callerKeyring);
    console.log(result.info);
    process.exit(0);
}
