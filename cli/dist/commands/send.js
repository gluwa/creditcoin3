"use strict";
var __awaiter = (this && this.__awaiter) || function (thisArg, _arguments, P, generator) {
    function adopt(value) { return value instanceof P ? value : new P(function (resolve) { resolve(value); }); }
    return new (P || (P = Promise))(function (resolve, reject) {
        function fulfilled(value) { try { step(generator.next(value)); } catch (e) { reject(e); } }
        function rejected(value) { try { step(generator["throw"](value)); } catch (e) { reject(e); } }
        function step(result) { result.done ? resolve(result.value) : adopt(result.value).then(fulfilled, rejected); }
        step((generator = generator.apply(thisArg, _arguments || [])).next());
    });
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.makeSendCommand = void 0;
const commander_1 = require("commander");
const api_1 = require("../api");
const tx_1 = require("../lib/tx");
const parsing_1 = require("../lib/parsing");
const keyring_1 = require("../lib/account/keyring");
function makeSendCommand() {
    const cmd = new commander_1.Command("send");
    cmd.description("Send CTC from an account");
    cmd.option("--use-ecdsa", "Use ECDSA signature scheme and a private key instead of a mnemonic phrase");
    cmd.option("-a, --amount [amount]", "Amount to send");
    cmd.option("-t, --to [to]", "Specify recipient address");
    cmd.action(sendAction);
    return cmd;
}
exports.makeSendCommand = makeSendCommand;
function sendAction(options) {
    return __awaiter(this, void 0, void 0, function* () {
        const { api } = yield (0, api_1.newApi)(options.url);
        const { amount, recipient } = parseOptions(options);
        const caller = yield (0, keyring_1.initCallerKeyring)(options);
        const tx = api.tx.balances.transfer(recipient, amount.toString());
        yield (0, tx_1.requireEnoughFundsToSend)(tx, caller.address, api, amount);
        const result = yield (0, tx_1.signSendAndWatch)(tx, api, caller);
        console.log(result.info);
        process.exit(0);
    });
}
function parseOptions(options) {
    const amount = (0, parsing_1.parseAmountOrExit)((0, parsing_1.requiredInput)(options.amount, "Failed to send CTC: Must specify an amount"));
    const recipient = (0, parsing_1.parseAddressOrExit)((0, parsing_1.requiredInput)(options.to, "Failed to send CTC: Must specify a recipient"));
    return { amount, recipient };
}
