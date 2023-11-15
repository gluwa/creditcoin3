import { Command, OptionValues } from 'commander';
import { getValidatorStatus, newApi, requireStatus } from '../../lib';
import { chill } from '../../lib/staking/chill';
import { initCallerKeyring } from '../../lib/account/keyring';
// import { getValidatorStatus, requireStatus } from '../utils/validatorStatus'

export function makeChillCommand() {
    const cmd = new Command('chill');
    cmd.description('Signal intention to stop validating from a Controller account');
    cmd.action(chillAction);
    return cmd;
}

async function chillAction(options: OptionValues) {
    const { api } = await newApi(options.url);

    const keyring = await initCallerKeyring(options);

    const address = keyring.address;

    const status = await getValidatorStatus(address, api);

    requireStatus(status, 'validating');

    console.log('Creating chill transaction...');

    const result = await chill(keyring, api);

    console.log(result.info);
    process.exit(0);
}
