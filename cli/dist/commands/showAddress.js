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
exports.makeShowAddressCommand = void 0;
const util_crypto_1 = require("@polkadot/util-crypto");
const commander_1 = require("commander");
const keyring_1 = require("../lib/account/keyring");
const util_crypto_2 = require("@polkadot/util-crypto");
const util_1 = require("@polkadot/util");
function makeShowAddressCommand() {
    const cmd = new commander_1.Command("show-address");
    cmd.description("Show account address");
    cmd.action(showAddressAction);
    return cmd;
}
exports.makeShowAddressCommand = makeShowAddressCommand;
function showAddressAction(options) {
    return __awaiter(this, void 0, void 0, function* () {
        yield (0, util_crypto_1.cryptoWaitReady)();
        const caller = yield (0, keyring_1.initCallerKeyring)(options);
        const substrateAddress = caller.address;
        const substrateAddressBytes = (0, util_crypto_2.decodeAddress)(substrateAddress);
        const evmAddress = (0, util_1.u8aToHex)(substrateAddressBytes).slice(0, 42);
        console.log("Account Substrate Address:", substrateAddress);
        console.log("Account EVM Address:", evmAddress);
        process.exit(0);
    });
}
