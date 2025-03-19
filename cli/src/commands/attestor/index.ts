import { Command } from 'commander';
import { noInputOption, urlOption } from '../options';
import { showAttestorBalanceActionCommand } from './balance';
import { makeAttestorWithdrawUnbondedCommand } from './withdrawUnbonded';
import { makeUnregisterAttestorCommand } from './unregisterAttestor';
import { showListAttestorsCommand } from './showListAttestors';
import { showClaimRewardsCommand } from './showUnclaimedRewards';
import { setPayeeCommand } from './setPayee';
import { makeShowAttestorStatusCommand } from './showAttestorStatus';
import { makeRegisterAttestorCommand } from './registerAttestor';
import { makeChillAttestorCommand } from './chill';
import { makeClaimRewardsCommand } from './claimRewards';

export function makeAttestorCommand() {
    const cmd = new Command('attestor');
    cmd.description('Interact with the attestor pallet of Creditcoin');
    cmd.addCommand(makeChillAttestorCommand());
    cmd.addCommand(makeClaimRewardsCommand());
    cmd.addCommand(makeRegisterAttestorCommand());
    cmd.addCommand(setPayeeCommand());
    cmd.addCommand(makeShowAttestorStatusCommand());
    cmd.addCommand(showListAttestorsCommand());
    cmd.addCommand(showClaimRewardsCommand());
    cmd.addCommand(makeUnregisterAttestorCommand());
    cmd.addCommand(makeAttestorWithdrawUnbondedCommand());
    cmd.addCommand(showAttestorBalanceActionCommand());

    cmd.commands.forEach((command) => {
        command.addOption(noInputOption);
        command.addOption(urlOption);
    });
    return cmd;
}
