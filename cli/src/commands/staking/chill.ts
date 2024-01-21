import { Command, OptionValues } from 'commander';
import { getValidatorStatus, newApi, requireStatus } from '../../lib';
import { chill } from '../../lib/staking/chill';
import { initCallerKeyring, initProxyKeyring } from '../../lib/account/keyring';

export function makeChillCommand() {
    const cmd = new Command('chill');
    cmd.description('Signal intention to stop validating from a Controller account');
    cmd.option('-p, --proxy', 'Whether to use a proxy account');
    cmd.option('-a, --address [address]', 'The address of the proxied account (use only with -p, --proxy');
    cmd.action(chillAction);
    return cmd;
}

async function chillAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const keyring = await initCallerKeyring(options);
    const proxy = await initProxyKeyring(options);

    const address = keyring.address;

    const status = await getValidatorStatus(address, api);

    requireStatus(status, 'validating');

    console.log('Creating chill transaction...');

    const result = await chill(keyring, api, proxy, options.address);

    console.log(result.info);
    process.exit(0);
}
