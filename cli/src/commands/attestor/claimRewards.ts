import { Command, OptionValues } from 'commander';
import { BN, newApi } from '../../lib';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../../lib/tx';
import { delegateAddress, initKeyring } from '../../lib/account/keyring';
import { proxyForOption } from '../options';
import { toCTCString } from '../../lib/balance';

export function makeClaimRewardsCommand() {
    const cmd = new Command('claim-rewards');
    cmd.description('Claim rewards the attestor has earned');
    cmd.addOption(proxyForOption);
    cmd.action(claimRewardsAction);
    return cmd;
}

async function claimRewardsAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const keyring = await initKeyring(options);
    const address = delegateAddress(keyring);

    const attestorRewards = await api.query.attestation.accumulatedRewards(address);
    if (attestorRewards.isNone) {
        console.log(`No rewards to claim for address ${address}`);
        process.exit(0);
    }
    const rewards = attestorRewards.unwrap();
    console.log(`Rewards available to claim: ${toCTCString(new BN(rewards), 4)} for address ${address}`);

    const claimRewardsAttestorTx = api.tx.attestation.claimRewards();

    await requireKeyringHasSufficientFunds(claimRewardsAttestorTx, keyring, api);
    const result = await signSendAndWatchCcKeyring(claimRewardsAttestorTx, api, keyring);
    console.log(result.info);
    process.exit(result.status);
}
