import { Command } from 'commander';
import { noInputOption, urlOption } from '../options';
import { showAttestorBalanceActionCommand } from './balance';
import { makeAttestorWithdrawUnbondedCommand } from './withdrawUnbonded';
import { makeUnregisterAttestorCommand } from './unregisterAttestor';
import { showAttestorsForCommand } from './showAttestorsFor';
import { makeShowAttestorStatusCommand } from './showAttestorStatus';
import { makeRegisterAttestorCommand } from './registerAttestor';
import { makeChillAttestorCommand } from './chill';

export function makeAttestorCommand() {
    const cmd = new Command('attestor');
    cmd.description('Interact with the attestor pallet of Creditcoin');
    cmd.addCommand(makeChillAttestorCommand());
    cmd.addCommand(makeRegisterAttestorCommand());
    cmd.addCommand(makeShowAttestorStatusCommand());
    cmd.addCommand(showAttestorsForCommand());
    cmd.addCommand(makeUnregisterAttestorCommand());
    cmd.addCommand(makeAttestorWithdrawUnbondedCommand());
    cmd.addCommand(showAttestorBalanceActionCommand());

    cmd.commands.forEach((command) => {
        command.addOption(noInputOption);
        command.addOption(urlOption);
    });
    return cmd;
}
