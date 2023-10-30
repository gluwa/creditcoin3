"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.makeNewSeedCommand = void 0;
const util_crypto_1 = require("@polkadot/util-crypto");
const commander_1 = require("commander");
function makeNewSeedCommand() {
    const cmd = new commander_1.Command("new");
    cmd.description("Create new seed phrase");
    cmd.option("-l, --length [word-length]", "Specify the amount of words");
    cmd.action(newSeedAction);
    return cmd;
}
exports.makeNewSeedCommand = makeNewSeedCommand;
function newSeedAction(options) {
    console.log("Creating new seed phrase...");
    const length = options.length ? parseLength(options.length) : 12;
    const seedPhrase = (0, util_crypto_1.mnemonicGenerate)(length);
    console.log("Seed phrase:", seedPhrase);
    process.exit(0);
}
function parseLength(length) {
    const parsed = parseInt(length, 10);
    if (parsed !== 12 &&
        parsed !== 15 &&
        parsed !== 18 &&
        parsed !== 21 &&
        parsed !== 24) {
        console.error("Failed to create new seed phrase: Invalid length, must be one of 12, 15, 18, 21 or 24");
        process.exit(1);
    }
    return parsed;
}
