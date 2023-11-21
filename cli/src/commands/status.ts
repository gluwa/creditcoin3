import { Command, OptionValues } from 'commander';
import { newApi } from '../lib';
import { parseBoolean, parseAddressOrExit } from '../lib/parsing';
import { getChainStatus, printChainStatus } from '../lib/chain/status';
import { getValidatorStatus, printValidatorStatus } from '../lib/staking/validatorStatus';

export function makeStatusCommand() {
    const cmd = new Command('status');
    cmd.description('Get staking status for an address');
    cmd.option('--validator [address]', 'Validator address to get status for');
    cmd.option('--chain', 'Show chain status');
    cmd.action(statusAction);
    return cmd;
}

async function statusAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const showValidatorStatus = parseBoolean(options.validator);
    let showChainStatus = parseBoolean(options.chain);

    if (!showValidatorStatus && !showChainStatus) {
        showChainStatus = true;
    }

    if (showChainStatus) {
        const chainStatus = await getChainStatus(api);
        printChainStatus(chainStatus);
    }

    if (showValidatorStatus) {
        const validator = parseAddressOrExit(options.validator);
        const validatorStatus = await getValidatorStatus(validator, api);
        console.log(`Validator ${validator}:`);
        await printValidatorStatus(validatorStatus, api);
    }

    process.exit(0);
}
