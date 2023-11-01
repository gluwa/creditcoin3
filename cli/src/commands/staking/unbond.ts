// import { getValidatorStatus, requireStatus } from '../utils/validatorStatus'

import { Command, OptionValues } from 'commander'
import { newApi } from '../../api'
import { ApiPromise } from '@polkadot/api'
import { BN } from '../../lib'
import { getBalance } from '../../lib/balance'
import { promptContinue } from '../../lib/interactive'
import { parseAmountOrExit, requiredInput } from '../../lib/parsing'
import { requireEnoughFundsToSend, signSendAndWatch } from '../../lib/tx'
import { initStashKeyring } from '../../lib/account/keyring'

export function makeUnbondCommand() {
    const cmd = new Command('unbond')
    cmd.description('Schedule a portion of the stash to be unlocked')
    cmd.option('-a, --amount [amount]', 'Amount to send')
    cmd.action(unbondAction)
    return cmd
}

async function unbondAction(options: OptionValues) {
    const { api } = await newApi(options.url)

    // const interactive = setInteractivity(options)

    const amount = parseAmountOrExit(
        requiredInput(
            options.amount,
            'Failed to unbond: Must specify an amount'
        )
    )

    // Build account
    const controller = await initStashKeyring(options)

    // TODO resupport status checks and unbonding max warning
    //
    // const controllerStatus = await getValidatorStatus(controller.address, api)
    // if (!controllerStatus.stash) {
    //     console.error(
    //         `Cannot unbond, ${controller.address} is not a controller account`
    //     )
    //     process.exit(1)
    // }
    // const stashStatus = await getValidatorStatus(controllerStatus.stash, api)
    // requireStatus(stashStatus, 'bonded')

    // // Check if amount specified exceeds total bonded funds
    // await checkIfUnbodingMax(controllerStatus.stash, amount, api, interactive)

    // Unbond transaction
    const tx = api.tx.staking.unbond(amount.toString())
    await requireEnoughFundsToSend(tx, controller.address, api)

    const result = await signSendAndWatch(tx, api, controller)

    console.log(result.info)
    process.exit(0)
}

async function checkIfUnbodingMax(
    address: string,
    unbondAmount: BN,
    api: ApiPromise,
    interactive: boolean
) {
    const balance = await getBalance(address, api)
    if (balance.bonded.lt(unbondAmount)) {
        console.error(
            'Warning: amount specified exceeds total bonded funds, will unbond all funds'
        )
        await promptContinue(interactive)
    }
}
