import { Command, OptionValues } from 'commander';
import { newApi } from '../../api';
import { initStashKeyring } from '../../lib/account/keyring';
import { parseHexStringOrExit } from '../../lib/parsing';
import { requireEnoughFundsToSend, signSendAndWatch } from '../../lib/tx';

export function makeSetKeysCommand() {
    const cmd = new Command('set-keys');
    cmd.description('Set session keys for a stash account');
    cmd.option('-k, --keys [keys]', 'Specify keys to set');
    cmd.option('-r, --rotate', 'Rotate and set new keys');

    cmd.action(setKeysAction);
    return cmd;
}

async function setKeysAction(options: OptionValues) {
    const { api } = await newApi(options.url);

    // Build account
    const stash = await initStashKeyring(options);

    let keys;
    if (!options.keys && !options.rotate) {
        console.log(
            'Must specify keys to set or generate new ones using the --rotate flag'
        );
        process.exit(1);
    } else if (options.keys && options.rotate) {
        console.error(
            'Must either specify keys or rotate to generate new ones, can not do both'
        );
        process.exit(1);
    } else if (options.rotate) {
        keys = (await api.rpc.author.rotateKeys()).toString();
    } else {
        keys = parseHexStringOrExit(options.keys);
    }

    const tx = api.tx.session.setKeys(keys, '');

    await requireEnoughFundsToSend(tx, stash.address, api);

    const result = await signSendAndWatch(tx, api, stash);

    console.log(result.info);

    process.exit(0);
}
