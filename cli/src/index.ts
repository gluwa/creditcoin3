// #!/usr/bin/env node
import { Command } from 'commander';
import { makeStatusCommand } from './commands/status';

const program = new Command();

program
    .addCommand(makeStatusCommand());

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

