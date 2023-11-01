// #!/usr/bin/env node
import { Command } from 'commander'
import { makeStatusCommand } from './commands/status'
import { makeNewSeedCommand } from './commands/newSeed'
import { makeShowAddressCommand } from './commands/showAddress'
import { makeBalanceCommand } from './commands/balance'
import { makeSendCommand } from './commands/send'
import { makeBondCommand } from './commands/staking/bond'
import { makeChillCommand } from './commands/staking/chill'
import { makeRotateKeysCommand } from './commands/session/rotateKeys'
import { makeSetKeysCommand } from './commands/staking/setKeys'
import { makeUnbondCommand } from './commands/staking/unbond'
import { makeValidateCommand } from './commands/staking/validate'

const program = new Command()

program
    .addCommand(makeStatusCommand())
    .addCommand(makeNewSeedCommand())
    .addCommand(makeShowAddressCommand())
    .addCommand(makeBalanceCommand())
    .addCommand(makeSendCommand())
    .addCommand(makeBondCommand())
    .addCommand(makeChillCommand())
    .addCommand(makeRotateKeysCommand())
    .addCommand(makeSetKeysCommand())
    .addCommand(makeUnbondCommand())
    .addCommand(makeValidateCommand())

program.commands.forEach((cmd) => {
    cmd.option('--no-input', 'Disable interactive prompts')
    cmd.option(
        '-u, --url [url]',
        'URL for the Substrate node',
        'ws://localhost:9944'
    )
})

program.parse(process.argv)
