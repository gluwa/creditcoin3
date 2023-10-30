"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
// #!/usr/bin/env node
const commander_1 = require("commander");
const status_1 = require("./commands/status");
const newSeed_1 = require("./commands/newSeed");
const showAddress_1 = require("./commands/showAddress");
const balance_1 = require("./commands/balance");
const send_1 = require("./commands/send");
const program = new commander_1.Command();
program
    .addCommand((0, status_1.makeStatusCommand)())
    .addCommand((0, newSeed_1.makeNewSeedCommand)())
    .addCommand((0, showAddress_1.makeShowAddressCommand)())
    .addCommand((0, balance_1.makeBalanceCommand)())
    .addCommand((0, send_1.makeSendCommand)());
program.commands.forEach((cmd) => {
    cmd.option("--no-input", "Disable interactive prompts");
    cmd.option("-u, --url [url]", "URL for the Substrate node", "ws://localhost:9944");
});
program.parse(process.argv);
// console.log(program.opts());
