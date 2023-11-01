import { cryptoWaitReady } from '@polkadot/util-crypto'
import { Command, OptionValues } from 'commander'
import { initCallerKeyring } from '../lib/account/keyring'

export function makeShowAddressCommand() {
    const cmd = new Command('show-address')
    cmd.description('Show account address')
    cmd.action(showAddressAction)
    cmd.option('--index [index]', 'Specify account index')
    return cmd
}

async function showAddressAction(options: OptionValues) {
    await cryptoWaitReady()

    const caller = await initCallerKeyring(options)

    console.log('Account address:', caller.address)

    process.exit(0)
}
