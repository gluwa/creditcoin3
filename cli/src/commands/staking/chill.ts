import { Command, OptionValues } from 'commander';
import { getValidatorStatus, newApi, requireStatus } from '../../lib';
import { chill } from '../../lib/staking/chill';
import { initKeyring } from '../../lib/account/keyring';
import { parseSubstrateAddress } from '../options';

export function makeChillCommand() {
    const cmd = new Command('chill');
    cmd.description('Signal intention to stop validating from a Controller account');
    cmd.option('-p, --proxy', 'Whether to use a proxy account');
    cmd.option('-a, --address [address]', 'The address that is being proxied', parseSubstrateAddress);
    cmd.action(chillAction);
    return cmd;
}

async function chillAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const keyring = await initKeyring(options);

    const address = keyring.pair.address;

    const status = await getValidatorStatus(address, api);

    requireStatus(status, 'validating');

    console.log('Creating chill transaction...');

    const result = await chill(keyring, api);

    console.log(result.info);
    process.exit(0);
}
