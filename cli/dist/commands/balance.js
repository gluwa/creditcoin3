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
exports.makeBalanceCommand = void 0;
const commander_1 = require("commander");
const api_1 = require("../api");
const balance_1 = require("../lib/balance");
const parsing_1 = require("../lib/parsing");
function makeBalanceCommand() {
    const cmd = new commander_1.Command("balance");
    cmd.description("Get balance of an account");
    cmd.option("-a, --address [address]", "Specify address to get balance of");
    cmd.option("--json", "Output as JSON");
    cmd.action(balanceAction);
    return cmd;
}
exports.makeBalanceCommand = makeBalanceCommand;
function balanceAction(options) {
    return __awaiter(this, void 0, void 0, function* () {
        const json = (0, parsing_1.parseBoolean)(options.json);
        const { api } = yield (0, api_1.newApi)(options.url);
        const address = (0, parsing_1.parseAddressOrExit)((0, parsing_1.requiredInput)(options.address, "Failed to show balance: Must specify an address"));
        const balance = yield (0, balance_1.getBalance)(address, api);
        (0, balance_1.logBalance)(balance, !json);
        process.exit(0);
    });
}
