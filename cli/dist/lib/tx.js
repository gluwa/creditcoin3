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
exports.requireEnoughFundsToSend = exports.canPay = exports.getTxFee = exports.TxStatus = exports.signSendAndWatch = void 0;
const balance_1 = require("./balance");
const _1 = require(".");
function signSendAndWatch(tx, api, signer) {
    return __awaiter(this, void 0, void 0, function* () {
        return new Promise((resolve, reject) => {
            console.log("Sending transaction...");
            let maybeUnsub;
            const unsubAndResolve = (result) => {
                if (maybeUnsub) {
                    maybeUnsub();
                }
                resolve(result);
            };
            // Sign and send with callback
            tx.signAndSend(signer, { nonce: -1 }, ({ status, dispatchError }) => {
                // Called every time the status changes
                if (status.isFinalized) {
                    const result = {
                        status: TxStatus.ok,
                        info: `Transaction included at blockHash ${status.asFinalized.toString()}`,
                    };
                    unsubAndResolve(result);
                }
                if (dispatchError) {
                    let blockHash = null;
                    if (status.isInBlock)
                        blockHash = status.asInBlock.toHex();
                    if (dispatchError.isModule) {
                        // for module errors, the section is indexed, lookup
                        const decoded = api.registry.findMetaError(dispatchError.asModule);
                        const { docs, name, section } = decoded;
                        const error = `${section}.${name}: ${docs.join(" ")}`;
                        const result = {
                            status: TxStatus.failed,
                            info: `Transaction failed with error: "${error}" ${blockHash ? "at block " + blockHash : ""}`,
                        };
                        unsubAndResolve(result);
                    }
                    else {
                        // Other, CannotLookup, BadOrigin, no extra info
                        const result = {
                            status: TxStatus.failed,
                            info: `Transaction failed with error: "${dispatchError.toString()}" ${blockHash ? "at block " + blockHash : ""}`,
                        };
                        unsubAndResolve(result);
                    }
                }
            })
                .then((unsub) => {
                maybeUnsub = unsub;
            })
                .catch((err) => {
                reject(err);
            });
        });
    });
}
exports.signSendAndWatch = signSendAndWatch;
// eslint-disable-next-line no-shadow
var TxStatus;
(function (TxStatus) {
    TxStatus[TxStatus["ok"] = 0] = "ok";
    TxStatus[TxStatus["failed"] = 1] = "failed";
})(TxStatus || (exports.TxStatus = TxStatus = {}));
function getTxFee(tx, callerAddress) {
    return __awaiter(this, void 0, void 0, function* () {
        const fee = yield tx.paymentInfo(callerAddress);
        return fee.partialFee.toBn();
    });
}
exports.getTxFee = getTxFee;
function canPay(balance, amount, existentialDeposit = new _1.BN(1)) {
    const availableBalance = balance.transferable;
    const availableAfter = availableBalance.sub(amount);
    return availableAfter.gte(existentialDeposit);
}
exports.canPay = canPay;
function requireEnoughFundsToSend(tx, address, api, amount = new _1.BN(0)) {
    return __awaiter(this, void 0, void 0, function* () {
        const balance = yield (0, balance_1.getBalance)(address, api);
        const txFee = yield getTxFee(tx, address);
        const totalCost = amount.add(txFee);
        if (!canPay(balance, totalCost)) {
            console.error(`Caller ${address} has insufficient funds to send the transaction (requires ${(0, balance_1.toCTCString)(totalCost)}); transaction cancelled.`);
            process.exit(1);
        }
    });
}
exports.requireEnoughFundsToSend = requireEnoughFundsToSend;
