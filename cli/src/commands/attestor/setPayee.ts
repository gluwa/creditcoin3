import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';
import { signSendAndWatchCcKeyring } from '../../lib/tx';
import { initKeyring } from '../../lib/account/keyring';
import { proxyForOption } from '../options';
import { inputOrDefault, parseChoiceOrExit } from '../../lib/parsing';

export function setPayeeCommand() {
    const cmd = new Command('set-payee-attestor');
    cmd.description('Set payee address for attestor, which will receive rewards on claim rewards');
    cmd.addOption(proxyForOption);
    cmd.option('-p, --payee [payee]', 'Specify payee address to set');
    cmd.action(setPayeeAction);
    return cmd;
}

async function setPayeeAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const keyring = await initKeyring(options);

    const payeeDestination = parsePayeeDestination(
        parseChoiceOrExit(inputOrDefault(options.payee, 'Staked'), ['Staked', 'Stash']),
    );

    const setPayeeAttestorTx = api.tx.attestation.setPayee(payeeDestination);

    const result = await signSendAndWatchCcKeyring(setPayeeAttestorTx, api, keyring);
    console.log(result.info);
    process.exit(result.status);
}

export type RewardDestination = 'Staked' | 'Stash';
export function parsePayeeDestination(rewardDestinationRaw: string): RewardDestination {
    // Capitalize first letter and lowercase the rest
    const rewardDestination =
        rewardDestinationRaw.charAt(0).toUpperCase() + rewardDestinationRaw.slice(1).toLowerCase();

    if (rewardDestination !== 'Stash' && rewardDestination !== 'Staked') {
        throw new Error("Invalid reward destination, must be one of 'Staked' or 'Stash'");
    } else {
        return rewardDestination;
    }
}
