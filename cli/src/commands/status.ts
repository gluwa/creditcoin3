import { Command, OptionValues } from 'commander';
import { newApi } from '../lib';
import { parseBoolean } from '../lib/parsing';
import { getValidatorStatus, printValidatorStatus } from '../lib/staking/validatorStatus';
import { substrateAddressOption } from './options';

export function makeStatusCommand() {
    const cmd = new Command('status');
    cmd.description('Get staking status for an address');
    cmd.addOption(substrateAddressOption);
    cmd.action(statusAction);
    return cmd;
}

async function statusAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const showValidatorStatus = parseBoolean(options.substrateAddress);

    if (showValidatorStatus) {
        const validatorAddr = options.substrateAddress as string;
        const validatorStatus = await getValidatorStatus(validatorAddr, api);
        console.log(`Validator ${validatorAddr}:`);
        await printValidatorStatus(validatorStatus, api);
    }

    process.exit(0);
}
