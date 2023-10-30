// #!/usr/bin/env node
import { Command } from 'commander';
import { makeStatusCommand } from './commands/status';
import { makeNewSeedCommand } from './commands/newSeed';
import { makeShowAddressCommand } from './commands/showAddress';
import { makeBalanceCommand } from './commands/balance';
import { makeSendCommand } from './commands/send';

const program = new Command();

program
    .addCommand(makeStatusCommand())
    .addCommand(makeNewSeedCommand())
    .addCommand(makeShowAddressCommand())
    .addCommand(makeBalanceCommand())
    .addCommand(makeSendCommand());

program.commands.forEach((cmd) => {
    cmd.option("--no-input", "Disable interactive prompts");
    cmd.option(
      "-u, --url [url]",
      "URL for the Substrate node",
      "ws://localhost:9944",
    );
  });

program.parse(process.argv);




// console.log(program.opts());

