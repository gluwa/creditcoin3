"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.requiredInput = exports.inputOrDefault = exports.parsePercentAsPerbillInternal = exports.parseHexStringInternal = exports.parseIntegerInternal = exports.parseBoolean = exports.parseChoiceInternal = exports.parseAmountInternal = exports.parseAddressInternal = exports.parseChoiceOrExit = exports.parsePercentAsPerbillOrExit = exports.parseIntegerOrExit = exports.parseHexStringOrExit = exports.parseAmountOrExit = exports.parseAddressOrExit = void 0;
const address_1 = require("@polkadot/util-crypto/address");
const _1 = require(".");
// Parse valid or exit with error
exports.parseAddressOrExit = parseOrExit(parseAddressInternal);
exports.parseAmountOrExit = parseOrExit(parseAmountInternal);
exports.parseHexStringOrExit = parseOrExit(parseHexStringInternal);
exports.parseIntegerOrExit = parseOrExit(parseIntegerInternal);
exports.parsePercentAsPerbillOrExit = parseOrExit(parsePercentAsPerbillInternal);
exports.parseChoiceOrExit = parseChoiceOrExitFn;
// A function that takes a parsing function and returns a new function that does tries to parse or prints the error and exits
function parseOrExit(parse) {
    return (input) => {
        try {
            return parse(input);
        }
        catch (e) {
            console.error(`Unable to parse input. ${e.message}`);
            process.exit(1);
        }
    };
}
function parseChoiceOrExitFn(input, choices) {
    try {
        return parseChoiceInternal(input, choices);
    }
    catch (e) {
        console.error(`Unable to parse input. ${e.message}`);
        process.exit(1);
    }
}
function parseAddressInternal(input) {
    try {
        (0, address_1.validateAddress)(input);
    }
    catch (e) {
        throw new Error(`Invalid address: ${e.message}`);
    }
    return input;
}
exports.parseAddressInternal = parseAddressInternal;
function parseAmountInternal(input) {
    try {
        const parsed = positiveBigNumberFromString(input);
        return new _1.BN(parsed.toString());
    }
    catch (e) {
        throw new Error(`Invalid amount: ${e.message}`);
    }
}
exports.parseAmountInternal = parseAmountInternal;
// Choices must be in Capitalized form: ['Staked', 'Stash', 'Controller']
function parseChoiceInternal(input, choices) {
    const styled = input.charAt(0).toUpperCase() + input.slice(1).toLowerCase();
    if (!choices.includes(styled)) {
        throw new Error(`Invalid choice: ${input}, must be one of ${choices.toString()}`);
    }
    return styled;
}
exports.parseChoiceInternal = parseChoiceInternal;
function parseBoolean(input) {
    return input ? true : false;
}
exports.parseBoolean = parseBoolean;
function parseIntegerInternal(input) {
    const float = Number.parseFloat(input);
    if (float % 1 !== 0) {
        throw new Error("Must be an integer");
    }
    const int = Number.parseInt(input, 10);
    return int;
}
exports.parseIntegerInternal = parseIntegerInternal;
function parseHexStringInternal(input) {
    if (!input.match(/^0x[\da-f]+$/i)) {
        throw new Error("Must be a valid hexadecimal number");
    }
    return input;
}
exports.parseHexStringInternal = parseHexStringInternal;
function parsePercentAsPerbillInternal(input) {
    if (input.match(/[^0-9.]/)) {
        throw new Error("Percent value must be a number");
    }
    const value = Number.parseFloat(input);
    if (value < 0 || value > 100) {
        throw new Error("Percent value must be between 0 and 100");
    }
    return Math.floor(value * 10000000);
}
exports.parsePercentAsPerbillInternal = parsePercentAsPerbillInternal;
function positiveBigNumberFromString(amount) {
    const parsedValue = (0, _1.parseUnits)(amount, 18);
    if (parsedValue === BigInt(0)) {
        throw new Error("Must be greater than 0");
    }
    if (parsedValue < BigInt(0)) {
        throw new Error("Must be a positive number");
    }
    return parsedValue;
}
function inputOrDefault(input, defaultValue) {
    if (input === undefined) {
        return defaultValue;
    }
    return input;
}
exports.inputOrDefault = inputOrDefault;
function requiredInput(input, message) {
    if (input === undefined) {
        console.error(message);
        process.exit(1);
    }
    return input;
}
exports.requiredInput = requiredInput;
