import { Command, OptionValues } from 'commander'
import { newApi } from '../../api'
import { bond, BN, MICROUNITS_PER_CTC, checkRewardDestination } from '../../lib'
import { initStashKeyring } from '../../lib/account/keyring'
import {
    toCTCString,
    getBalance,
    parseCTCString,
    AccountBalance,
    printBalance,
} from '../../lib/balance'
import {
    promptContinue,
    promptContinueOrSkip,
    setInteractivity,
} from '../../lib/interactive'
import {
    parseAmountOrExit,
    requiredInput,
    parseChoiceOrExit,
    inputOrDefault,
    parsePercentAsPerbillOrExit,
    parseBoolean,
} from '../../lib/parsing'
import { StakingPalletValidatorPrefs } from '../../lib/staking/validate'
import {
    TxStatus,
    requireEnoughFundsToSend,
    signSendAndWatch,
} from '../../lib/tx'
import { percentFromPerbill } from '../../lib/perbill'

export function makeWizardCommand() {
    const cmd = new Command('wizard')
    cmd.description(
        'Run the validator setup wizard. Only requires funded stash and controller accounts.'
    )
    cmd.option(
        '-r, --reward-destination [reward-destination]',
        'Specify reward destination account to use for new account'
    )
    cmd.option('-a, --amount [amount]', 'Amount to bond')
    cmd.option('--commission [commission]', 'Specify commission for validator')
    cmd.option(
        '--blocked',
        'Specify if validator is blocked for new nominations'
    )
    cmd.action(async (options: OptionValues) => {
        console.log('🧙 Running staking wizard...')

        const { amount, rewardDestination, commission, blocked, interactive } =
            parseOptions(options)

        // Create new API instance
        const { api } = await newApi(options.url)

        // Generate stash keyring
        const stashKeyring = await initStashKeyring(options)
        const stashAddress = stashKeyring.address

        // Validate prefs
        const preferences: StakingPalletValidatorPrefs = { commission, blocked }

        // Node settings
        const nodeUrl: string = options.url
            ? options.url
            : 'ws://localhost:9944'

        // State parameters being used
        console.log('Using the following parameters:')
        console.log(`💰 Stash account: ${stashAddress}`)
        console.log(`🪙 Amount to bond: ${toCTCString(amount)}`)
        console.log(`🎁 Reward destination: ${rewardDestination}`)
        console.log(`📡 Node URL: ${nodeUrl}`)
        console.log(
            `💸 Commission: ${percentFromPerbill(commission).toString()}`
        )
        console.log(`🔐 Blocked: ${blocked ? 'Yes' : 'No'}`)

        // Prompt continue
        await promptContinue(interactive)

        // get balances.
        const stashBalance = await getBalance(stashAddress, api)

        // ensure they have enough fee's and balance to cover the wizard.
        const grosslyEstimatedFee = parseCTCString('2')

        const amountWithFee = amount.add(grosslyEstimatedFee)
        checkStashBalance(stashAddress, stashBalance, amountWithFee)

        const bondExtra: boolean = checkIfAlreadyBonded(stashBalance)

        if (bondExtra) {
            console.log(
                '⚠️  Warning: Stash account already bonded. This will increase the amount bonded.'
            )
            if (
                await promptContinueOrSkip(
                    `Continue or skip bonding extra funds?`,
                    interactive
                )
            ) {
                checkStashBalance(stashAddress, stashBalance, amount)
                // Bond extra
                console.log('Sending bond transaction...')
                const bondTxResult = await bond(
                    stashKeyring,
                    amount,
                    rewardDestination,
                    api,
                    bondExtra
                )
                console.log(bondTxResult.info)
                if (bondTxResult.status === TxStatus.failed) {
                    console.log('Bond transaction failed. Exiting.')
                    process.exit(1)
                }
            }
        } else {
            // Bond
            console.log('Sending bond transaction...')
            const bondTxResult = await bond(
                stashKeyring,
                amount,
                rewardDestination,
                api
            )
            console.log(bondTxResult.info)
            if (bondTxResult.status === TxStatus.failed) {
                console.log('Bond transaction failed. Exiting.')
                process.exit(1)
            }
        }

        // Rotate keys
        console.log('Generating new session keys on node...')
        const newKeys = (await api.rpc.author.rotateKeys()).toString()
        console.log('New node session keys:', newKeys)

        // Set keys
        console.log('Creating setKeys transaction...')
        const setKeysTx = api.tx.session.setKeys(newKeys, '')

        // Validate
        console.log('Creating validate transaction...')
        const validateTx = api.tx.staking.validate(preferences)

        // Send transactions
        console.log('Sending setKeys and validate transactions...')
        const txs = [setKeysTx, validateTx]

        const batchTx = api.tx.utility.batchAll(txs)
        await requireEnoughFundsToSend(batchTx, stashAddress, api)

        const batchResult = await signSendAndWatch(batchTx, api, stashKeyring)

        console.log(batchResult.info)

        // // Inform process
        console.log('🧙 Validator wizard completed successfully!')
        console.log('Your validator should appear on the waiting queue.')

        process.exit(0)
    })
    return cmd
}

function checkControllerBalance(
    address: string,
    balance: AccountBalance,
    amount: BN
) {
    if (balance.transferable.lt(amount)) {
        console.log(
            'Controller account does not have enough funds to pay transaction fees'
        )
        printBalance(balance)
        console.log(
            `Please send at least ${toCTCString(
                amount
            )} to controller address ${address} and try again.`
        )
        process.exit(1)
    }
}

function checkStashBalance(
    address: string,
    balance: AccountBalance,
    amount: BN
) {
    if (balance.transferable.lt(amount)) {
        console.log(
            `Stash account does not have enough funds to bond ${toCTCString(
                amount
            )}`
        )
        printBalance(balance)
        console.log(
            `Please send funds to stash address ${address} and try again.`
        )
        process.exit(1)
    }
}

function checkIfAlreadyBonded(balance: AccountBalance) {
    if (balance.bonded.gt(new BN(0))) {
        return true
    } else {
        return false
    }
}

function parseOptions(options: OptionValues) {
    const interactive = setInteractivity(options)

    const amount = parseAmountOrExit(
        requiredInput(
            options.amount,
            'Failed to setup wizard: Bond amount required'
        )
    )
    if (amount.lt(new BN(1).mul(new BN(MICROUNITS_PER_CTC)))) {
        console.log(
            'Failed to setup wizard: Bond amount must be at least 1 CTC'
        )
        process.exit(1)
    }

    const rewardDestination = checkRewardDestination(
        parseChoiceOrExit(inputOrDefault(options.rewardDestination, 'Staked'), [
            'Staked',
            'Stash',
            'Controller',
        ])
    )

    const commission = parsePercentAsPerbillOrExit(
        inputOrDefault(options.commission, '0')
    )

    const blocked = parseBoolean(options.blocked)

    return { amount, rewardDestination, commission, blocked, interactive }
}
