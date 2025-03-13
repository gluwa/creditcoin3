import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../../lib/tx';
import { initKeyring } from '../../lib/account/keyring';
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

    const attestorRewards = await api.query.attestation.accumulatedRewards(keyring.pair.address);
    if (attestorRewards.isNone) {
        console.log(`No rewards to claim for address ${keyring.pair.address}`);
        process.exit(0);
    }
    const rewards = attestorRewards.unwrap();
    console.log(`Rewards available to claim: ${rewards.toString()} for address ${keyring.pair.address}`);

    const claimRewardsAttestorTx = api.tx.attestation.claimRewards();

    await requireKeyringHasSufficientFunds(claimRewardsAttestorTx, keyring, api);
    const result = await signSendAndWatchCcKeyring(claimRewardsAttestorTx, api, keyring);
    console.log(result.info);
    process.exit(result.status);
}
