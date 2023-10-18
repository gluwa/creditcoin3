"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
// #!/usr/bin/env node
const commander_1 = require("commander");
const status_1 = require("./commands/status");
const program = new commander_1.Command();
program
    .addCommand((0, status_1.makeStatusCommand)());
program.commands.forEach((cmd) => {
    cmd.option("--no-input", "Disable interactive prompts");
    cmd.option("-u, --url [url]", "URL for the Substrate node", "ws://localhost:9944");
});
program.parse(process.argv);
// console.log(program.opts());
