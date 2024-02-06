import { Command, OptionValues } from 'commander';
import { getValidatorStatus, newApi, requireStatus } from '../../lib';
import { chill } from '../../lib/staking/chill';
import { initKeyring, delegateAddress } from '../../lib/account/keyring';
import { useProxyOption } from '../options';

export function makeChillCommand() {
    const cmd = new Command('chill');
    cmd.description('Signal intention to stop validating from a Controller account');
    cmd.addOption(useProxyOption);
    cmd.action(chillAction);
    return cmd;
}

async function chillAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const keyring = await initKeyring(options);

    const address = delegateAddress(keyring);

    const status = await getValidatorStatus(address, api);

    requireStatus(status, 'validating');

    console.log('Creating chill transaction...');

    const result = await chill(keyring, api);

    console.log(result.info);
    process.exit(0);
}
