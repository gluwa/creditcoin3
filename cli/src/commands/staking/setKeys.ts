import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';
import { parseHexStringOrExit } from '../../lib/parsing';
import { requireKeyringHasSufficientFunds, signSendAndWatchCcKeyring } from '../../lib/tx';
import { initKeyring } from '../../lib/account/keyring';
import { proxyForOption } from '../options';

export function makeSetKeysCommand() {
    const cmd = new Command('set-keys');
    cmd.description('Set session keys for a bonded account');
    cmd.option('-k, --keys [keys]', 'Specify keys to set');
    cmd.option('-r, --rotate', 'Rotate and set new keys');
    cmd.addOption(proxyForOption);
    cmd.action(setKeysAction);
    return cmd;
}

async function setKeysAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);

    // Build account
    const keyring = await initKeyring(options);

    let keys;
    if (!options.keys && !options.rotate) {
        console.log('Must specify keys to set or generate new ones using the --rotate flag');
        process.exit(1);
    } else if (options.keys && options.rotate) {
        console.error('Must either specify keys or rotate to generate new ones, can not do both');
        process.exit(1);
    } else if (options.rotate) {
        keys = (await api.rpc.author.rotateKeys()).toString();
    } else {
        keys = parseHexStringOrExit(options.keys);
    }

    const tx = api.tx.session.setKeys(keys, '');

    await requireKeyringHasSufficientFunds(tx, keyring, api);

    const result = await signSendAndWatchCcKeyring(tx, api, keyring);

    console.log(result.info);

    process.exit(result.status);
}
