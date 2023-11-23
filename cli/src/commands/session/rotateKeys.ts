import { Command, OptionValues } from 'commander';
import { newApi } from '../../lib';

export function makeRotateKeysCommand() {
    const cmd = new Command('rotate-keys');
    cmd.description(`Rotate session keys for a specified node. While it does not require an account, it does require access to the node's unsafe RPCs, either by enabling external calls or by running this CLI tool in the same machine as the node.`);
    cmd.action(rotateKeysAction);
    return cmd;
}

async function rotateKeysAction(options: OptionValues) {
    const { api } = await newApi(options.url as string);
    const newKeys = await api.rpc.author.rotateKeys();
    console.log('New keys: ' + newKeys.toString());
    process.exit(0);
}
