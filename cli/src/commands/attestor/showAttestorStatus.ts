import { Command, OptionValues } from 'commander';
import { getValidatorStatus, newApi, requireStatus } from '../../lib';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../../lib/tx';
import { initKeyring, delegateAddress } from '../../lib/account/keyring';
import { proxyForOption } from '../options';
import { substrateAddressOption } from '../options';

export function makeClaimRewardsCommand() {
    const cmd = new Command('show-unclaimed-rewards');
    cmd.description('Show unclaimed rewards that attestor has earned');
    cmd.addOption(substrateAddressOption.makeOptionMandatory());
    cmd.option(
        '-c, --chain [chain]',
        'Specify chain key to register attestor for',
    );
    cmd.action(showUnclaimedRewardsAction);
    return cmd;
}

async function showUnclaimedRewardsAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const address = options.substrateAddress as string;
    const chainKey = options.chain as string;

    const activeAttestors = await api.query.attestation.activeAttestors(chainKey);
    for (let i = 0; i < activeAttestors.length; i++) {
        if (activeAttestors[i].toString() === address) {
            console.log(`Address ${address} status is Elected`);
            process.exit(0);
        }
    }

    const attestor = await api.query.attestation.attestors(chainKey, address);
    if (attestor.isNone) {
        console.log(`Address ${address} is not an attestor`);
        process.exit(0);
    }

    const status = attestor.unwrap().status;
    if (status.isActive) {
        console.log(`Address ${address} status is Active`);
        process.exit(0);
    }
    console.log(`Address ${address} status is Chill`);
    process.exit(0);
}
