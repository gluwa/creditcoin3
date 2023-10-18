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
exports.makeStatusCommand = void 0;
const commander_1 = require("commander");
const api_1 = require("../api");
function makeStatusCommand() {
    const cmd = new commander_1.Command("status");
    cmd.description("Get chain status");
    cmd.action(statusAction);
    return cmd;
}
exports.makeStatusCommand = makeStatusCommand;
function statusAction(options) {
    return __awaiter(this, void 0, void 0, function* () {
        const { api } = yield (0, api_1.newApi)(options.url);
        const bestBlock = yield api.rpc.chain.getBlock();
        const blockNumber = bestBlock.block.header.number.toNumber();
        console.log(`Best block number: ${blockNumber}`);
        process.exit(0);
    });
}
