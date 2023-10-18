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
exports.newApi = void 0;
const lib_1 = require("./lib");
const util_crypto_1 = require("@polkadot/util-crypto");
// Create new API instance
function newApi(url = "ws://localhost:9944") {
    return __awaiter(this, void 0, void 0, function* () {
        const ccApi = yield (0, lib_1.creditcoinApi)(url.trim(), true);
        yield (0, util_crypto_1.cryptoWaitReady)();
        return ccApi;
    });
}
exports.newApi = newApi;
