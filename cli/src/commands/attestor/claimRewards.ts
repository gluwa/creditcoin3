import { Command, OptionValues } from 'commander';
import { getValidatorStatus, newApi, requireStatus } from '../../lib';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../../lib/tx';
import { initKeyring, delegateAddress } from '../../lib/account/keyring';
import { proxyForOption } from '../options';

export function makeClaimRewardsCommand() {
    const cmd = new Command('claim-rewards-attestor');
    cmd.description('Claim rewards that attestor has earned');
    cmd.addOption(proxyForOption);
    cmd.action(claimRewardsAction);
    return cmd;
}

async function claimRewardsAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const keyring = await initKeyring(options);

    const claimRewardsAttestorTx = api.tx.attestation.claimRewards();

    await requireKeyringHasSufficientFunds(claimRewardsAttestorTx, keyring, api);
    const result = await signSendAndWatchCcKeyring(claimRewardsAttestorTx, api, keyring);
    console.log(result.info);
    process.exit(result.status);
}
