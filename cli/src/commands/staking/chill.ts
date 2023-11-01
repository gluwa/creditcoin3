import { Command, OptionValues } from 'commander'
import { newApi } from '../../api'
import { initStashKeyring } from '../../lib/account/keyring'
import { chill } from '../../lib/staking/chill'
// import { getValidatorStatus, requireStatus } from '../utils/validatorStatus'

export function makeChillCommand() {
    const cmd = new Command('chill')
    cmd.description(
        'Signal intention to stop validating from a Controller account'
    )
    cmd.action(chillAction)
    return cmd
}

async function chillAction(options: OptionValues) {
    const { api } = await newApi(options.url)

    const keyring = await initStashKeyring(options)

    // TODO resupport validator status check
    //
    // const address = keyring.address

    // const status = await getValidatorStatus(address, api)

    // if (!status.stash) {
    //     console.error(`Cannot chill, ${address} is not staked`)
    //     process.exit(1)
    // }
    // const stashStatus = await getValidatorStatus(status.stash, api)

    // requireStatus(stashStatus, 'validating')

    console.log('Creating chill transaction...')

    const result = await chill(keyring, api)

    console.log(result.info)
    process.exit(0)
}
