import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../../lib/tx';
import { initKeyring } from '../../lib/account/keyring';
import { proxyForOption } from '../options';

export function makeChillAttestorCommand() {
    const cmd = new Command('chill-attestor');
    cmd.description('Chill attestor');
    cmd.addOption(proxyForOption);
    cmd.option(
        '-c, --chain [chain]',
        'chain key to chill',
    );
    cmd.option(
        '-a, --attestor [attestor]',
        'Specify attestor account to unregister',
    );
    cmd.action(chillAction);
    return cmd;
}

async function chillAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const chainKey = options.chain as string;
    const attestor = options.attestor as string;

    const keyring = await initKeyring(options);

    const attestorByChain = await api.query.attestation.attestors(chainKey, attestor);
    if (attestorByChain.isNone) {
        console.log(`There is not attestor ${attestor} for chain ${chainKey}`);
        process.exit(0);
    }
    const attestorByChainValue = attestorByChain.unwrap();

    if (attestorByChainValue.stash.toString() !== keyring.pair.address) {
        console.log(`Attestor ${attestor} is not owned by the keyring account ${keyring.pair.address}`);
        process.exit(0);
    }

    if (attestorByChainValue.status.isIdle) {
        console.log(`Attestor ${attestor} is already chilled`);
        process.exit(0);
    }

    const chillAttestorTx = api.tx.attestation.chill(chainKey, attestor);

    await requireKeyringHasSufficientFunds(chillAttestorTx, keyring, api);
    const result = await signSendAndWatchCcKeyring(chillAttestorTx, api, keyring);
    console.log(result.info);
    process.exit(result.status);
}
