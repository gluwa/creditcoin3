#!/usr/bin/env node
import { Command } from 'commander';
import { makeStatusCommand } from './commands/status';
import { makeNewSeedCommand } from './commands/newSeed';
import { makeShowAddressCommand } from './commands/showAddress';
import { makeBalanceCommand } from './commands/balance';
import { makeSendCommand } from './commands/send';
import { makeBondCommand } from './commands/staking/bond';
import { makeChillCommand } from './commands/staking/chill';
import { makeRotateKeysCommand } from './commands/session/rotateKeys';
import { makeSetKeysCommand } from './commands/staking/setKeys';
import { makeUnbondCommand } from './commands/staking/unbond';
import { makeValidateCommand } from './commands/staking/validate';
import { makeDistributeRewardsCommand } from './commands/staking/distribute';
import { makeWithdrawUnbondedCommand } from './commands/staking/withdraw';
import { makeWizardCommand } from './commands/staking/wizard';
import { makeEvmCommand } from './commands/evm';
import { makeConvertAddressCommand } from './commands/convertAddress';
import { makeProxyCommands } from './commands/proxy/commands';
import { urlOption } from './commands/options';

const program = new Command();

// Add commands. WARNING: DO NOT ADD SUB COMMANDS HERE!
program
    .addCommand(makeBalanceCommand())
    .addCommand(makeBondCommand())
    .addCommand(makeChillCommand())
    .addCommand(makeConvertAddressCommand())
    .addCommand(makeDistributeRewardsCommand())
    .addCommand(makeNewSeedCommand())
    .addCommand(makeRotateKeysCommand())
    .addCommand(makeSendCommand())
    .addCommand(makeSetKeysCommand())
    .addCommand(makeShowAddressCommand())
    .addCommand(makeStatusCommand())
    .addCommand(makeUnbondCommand())
    .addCommand(makeValidateCommand())
    .addCommand(makeWithdrawUnbondedCommand())
    .addCommand(makeWizardCommand());

program.commands.forEach((cmd) => {
    cmd.option('--no-input', 'Disable interactive prompts');
    cmd.addOption(urlOption);
});

// Add Subcommands. WARNING: SUB COMMANDS SHOULD GO ONLY HERE
// b/c if --url option is specified for the top-level command,
// e.g. `proxy` and then also on the sub-command, e.g. `proxy add`
// it stops working. The reason is the default value from one of these options
// (I think the 2nd one) shadows the actual value specified on the command line
program.addCommand(makeEvmCommand());
program.addCommand(makeProxyCommands());
program.parse(process.argv);
