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
exports.creditcoinApi = void 0;
const api_1 = require("@polkadot/api");
const creditcoinApi = (wsUrl, noInitWarn = false) => __awaiter(void 0, void 0, void 0, function* () {
    const provider = new api_1.WsProvider(wsUrl);
    const api = yield api_1.ApiPromise.create({ provider, noInitWarn });
    return {
        api,
        // extrinsics: extrinsics(api),
        // utils: utils(api)
    };
});
exports.creditcoinApi = creditcoinApi;
