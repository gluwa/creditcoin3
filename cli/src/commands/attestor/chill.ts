import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../../lib/tx';
import { delegateAddress, initKeyring } from '../../lib/account/keyring';
import { proxyForOption, chainKeyOption, attestorAddressOption } from '../options';

export function makeChillAttestorCommand() {
    const cmd = new Command('chill');
    cmd.description('Chill attestor');
    cmd.addOption(proxyForOption);
    cmd.addOption(chainKeyOption.makeOptionMandatory());
    cmd.addOption(attestorAddressOption.makeOptionMandatory());
    cmd.action(chillAction);
    return cmd;
}

async function chillAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    const chainKey = options.chain as string;
    const attestor = options.attestor as string;
    const proxyFor = options.proxyFor as boolean;

    const keyring = await initKeyring(options);

    const attestorByChain = await api.query.attestation.attestors(chainKey, attestor);
    if (attestorByChain.isNone) {
        console.log(`There is not attestor ${attestor} for chain ${chainKey}`);
        process.exit(1);
    }
    const attestorByChainValue = attestorByChain.unwrap();
    const address = delegateAddress(keyring);

    if (attestorByChainValue.stash.toString() !== address) {
        console.log(`Attestor ${attestor} is not owned by the keyring account ${address}`);
        process.exit(1);
    }

    if (attestorByChainValue.status.isIdle) {
        console.log(`Attestor ${attestor} is already chilled`);
        process.exit(1);
    }

    const chillAttestorTx = api.tx.attestation.chill(chainKey, attestor);

    await requireKeyringHasSufficientFunds(chillAttestorTx, keyring, api);
    const result = await signSendAndWatchCcKeyring(chillAttestorTx, api, keyring);
    console.log(result.info);
    process.exit(result.status);
}
