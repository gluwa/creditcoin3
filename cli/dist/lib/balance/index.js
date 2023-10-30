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
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.checkAmount = exports.printJsonBalance = exports.printBalance = exports.logBalance = exports.getBalance = exports.readAmountFromHex = exports.readAmount = exports.toCTCString = exports.parseCTCString = exports.MICROUNITS_PER_CTC = void 0;
const __1 = require("..");
const cli_table3_1 = __importDefault(require("cli-table3"));
exports.MICROUNITS_PER_CTC = new __1.BN("1000000000000000000");
function parseCTCString(amount) {
    try {
        const parsed = positiveBigNumberFromString(amount);
        return new __1.BN(parsed.toString());
    }
    catch (e) {
        console.error(`Unable to parse CTC amount: ${amount}`);
        process.exit(1);
    }
}
exports.parseCTCString = parseCTCString;
function toCTCString(amount, decimals = 18) {
    const CTC = amount.div(exports.MICROUNITS_PER_CTC);
    const remainder = amount.mod(exports.MICROUNITS_PER_CTC);
    const remainderString = remainder
        .toString()
        .padStart(18, "0")
        .slice(0, decimals);
    return `${CTC.toString()}.${remainderString} CTC`;
}
exports.toCTCString = toCTCString;
function readAmount(amount) {
    return new __1.BN(amount);
}
exports.readAmount = readAmount;
function readAmountFromHex(amount) {
    return new __1.BN(amount.slice(2), 16);
}
exports.readAmountFromHex = readAmountFromHex;
function getBalance(address, api) {
    var _a;
    return __awaiter(this, void 0, void 0, function* () {
        const balacesAll = yield getBalancesAll(address, api);
        const stakingInfo = yield getStakingInfo(address, api);
        const balance = {
            address,
            transferable: balacesAll.availableBalance,
            bonded: ((_a = stakingInfo === null || stakingInfo === void 0 ? void 0 : stakingInfo.stakingLedger.active) === null || _a === void 0 ? void 0 : _a.unwrap()) || new __1.BN(0),
            locked: balacesAll.lockedBalance,
            total: balacesAll.freeBalance.add(balacesAll.reservedBalance),
            unbonding: calcUnbonding(stakingInfo),
        };
        return balance;
    });
}
exports.getBalance = getBalance;
function getBalancesAll(address, api) {
    return __awaiter(this, void 0, void 0, function* () {
        const balance = yield api.derive.balances.all(address);
        return balance;
    });
}
function getStakingInfo(address, api) {
    return __awaiter(this, void 0, void 0, function* () {
        const stakingInfo = yield api.derive.staking.account(address);
        return stakingInfo;
    });
}
function calcUnbonding(stakingInfo) {
    if (!(stakingInfo === null || stakingInfo === void 0 ? void 0 : stakingInfo.unlocking)) {
        return new __1.BN(0);
    }
    const filtered = stakingInfo.unlocking
        .filter(({ remainingEras, value }) => value.gt(new __1.BN(0)) && remainingEras.gt(new __1.BN(0)))
        .map((unlock) => unlock.value);
    const unbonding = filtered.reduce((total, value) => total.iadd(value), new __1.BN(0));
    return unbonding;
}
function logBalance(balance, human = true) {
    if (human) {
        printBalance(balance);
    }
    else {
        printJsonBalance(balance);
    }
}
exports.logBalance = logBalance;
function printBalance(balance) {
    const table = new cli_table3_1.default({});
    table.push(["Transferable", toCTCString(balance.transferable, 4)], ["Locked", toCTCString(balance.locked, 4)], ["Bonded", toCTCString(balance.bonded, 4)], ["Unbonding", toCTCString(balance.unbonding, 4)], ["Total", toCTCString(balance.total, 4)]);
    console.log(`Address: ${balance.address}`);
    console.log(table.toString());
}
exports.printBalance = printBalance;
function printJsonBalance(balance) {
    const jsonBalance = {
        balance: {
            address: balance.address,
            transferable: balance.transferable.toString(),
            bonded: balance.bonded.toString(),
            locked: balance.locked.toString(),
            unbonding: balance.unbonding.toString(),
            total: balance.total.toString(),
        },
    };
    console.log(JSON.stringify(jsonBalance, null, 2));
}
exports.printJsonBalance = printJsonBalance;
function checkAmount(amount) {
    if (!amount) {
        console.log("Must specify amount to bond");
        process.exit(1);
    }
    else {
        if (amount.lt(new __1.BN(1).mul(exports.MICROUNITS_PER_CTC))) {
            console.log("Bond amount must be at least 1 CTC");
            process.exit(1);
        }
    }
}
exports.checkAmount = checkAmount;
function positiveBigNumberFromString(amount) {
    const parsedValue = (0, __1.parseUnits)(amount, 18);
    if (parsedValue === BigInt(0)) {
        console.error("Failed to parse amount, must be greater than 0");
        process.exit(1);
    }
    if (parsedValue < BigInt(0)) {
        console.error("Failed to parse amount, must be a positive number");
        process.exit(1);
    }
    return parsedValue;
}
