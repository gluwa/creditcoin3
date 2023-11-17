import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';
import { initCallerKeyring } from 'src/lib/account/keyring';

export function makeInsertKeysCommand() {
    const cmd = new Command('insert-keys');
    cmd.description('Insert session keys into a specified node');
    cmd.action(insertKeysAction);
    return cmd;
}

async function insertKeysAction(options: OptionValues) {
    const { api } = await newApi(options.url);
    const keyring = initCallerKeyring(options);
}
