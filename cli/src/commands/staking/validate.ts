import { Command, OptionValues } from 'commander'
import { newApi } from '../../api'
import {
    parsePercentAsPerbillOrExit,
    inputOrDefault,
    parseBoolean,
} from '../../lib/parsing'
import { initStashKeyring } from '../../lib/account/keyring'
import {
    StakingPalletValidatorPrefs,
    validate,
} from '../../lib/staking/validate'

export function makeValidateCommand() {
    const cmd = new Command('validate')
    cmd.description('Signal intention to validate from a Stash account')
    cmd.option(
        '--commission [commission]',
        'Specify commission for validator in percent'
    )
    cmd.option(
        '--blocked',
        'Specify if validator is blocked for new nominations'
    )
    cmd.action(validateAction)
    return cmd
}

async function validateAction(options: OptionValues) {
    const { api } = await newApi(options.url)

    const account = await initStashKeyring(options)

    // Default commission is 0%
    const commission = parsePercentAsPerbillOrExit(
        inputOrDefault(options.commission, '0')
    )

    const blocked = parseBoolean(options.blocked)

    const preferences: StakingPalletValidatorPrefs = { commission, blocked }

    console.log('Creating validate transaction...')

    const result = await validate(account, preferences, api)

    console.log(result.info)
    process.exit(0)
}
