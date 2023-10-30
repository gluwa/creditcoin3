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
exports.validateECDSAKey = exports.initKeyringFromEnvOrPrompt = exports.initCallerKeyring = exports.initControllerKeyring = exports.initStashKeyring = exports.initECDSAKeyringPairFromPK = exports.initKeyringPair = void 0;
const util_crypto_1 = require("@polkadot/util-crypto");
const __1 = require("..");
const prompts_1 = __importDefault(require("prompts"));
const error_1 = require("../error");
function initKeyringPair(seed) {
    const keyring = new __1.Keyring({ type: "sr25519" });
    const pair = keyring.addFromUri(`${seed}`);
    return pair;
}
exports.initKeyringPair = initKeyringPair;
function initECDSAKeyringPairFromPK(pk) {
    const keyring = new __1.Keyring({ type: "ecdsa" });
    const pair = keyring.addFromUri(`${pk}`);
    return pair;
}
exports.initECDSAKeyringPairFromPK = initECDSAKeyringPairFromPK;
function initStashKeyring(options) {
    return __awaiter(this, void 0, void 0, function* () {
        try {
            return yield initKeyringFromEnvOrPrompt("CC_STASH_SECRET", "stash", options);
        }
        catch (e) {
            console.error((0, error_1.getErrorMessage)(e));
            process.exit(1);
        }
    });
}
exports.initStashKeyring = initStashKeyring;
function initControllerKeyring(options) {
    return __awaiter(this, void 0, void 0, function* () {
        try {
            return yield initKeyringFromEnvOrPrompt("CC_CONTROLLER_SECRET", "controller", options);
        }
        catch (e) {
            console.error((0, error_1.getErrorMessage)(e));
            process.exit(1);
        }
    });
}
exports.initControllerKeyring = initControllerKeyring;
function initCallerKeyring(options) {
    return __awaiter(this, void 0, void 0, function* () {
        try {
            return yield initKeyringFromEnvOrPrompt("CC_SECRET", "caller", options);
        }
        catch (e) {
            console.error((0, error_1.getErrorMessage)(e));
            process.exit(1);
        }
    });
}
exports.initCallerKeyring = initCallerKeyring;
function initKeyringFromEnvOrPrompt(envVar, accountRole, options) {
    return __awaiter(this, void 0, void 0, function* () {
        const interactive = options.input;
        const ecdsa = options.ecdsa;
        const inputName = ecdsa ? "private key" : "seed phrase";
        const validateInput = ecdsa ? validateECDSAKey : util_crypto_1.mnemonicValidate;
        const generateKeyring = ecdsa ? initECDSAKeyringPairFromPK : initKeyringPair;
        if (!interactive && !process.env[envVar]) {
            throw new Error(`Error: Must specify a ${inputName} for the ${accountRole} account in the environment variable ${envVar} or use an interactive shell.`);
        }
        if (typeof process.env[envVar] === "string") {
            const input = getStringFromEnvVar(process.env[envVar]);
            if (validateInput(input)) {
                return generateKeyring(input);
            }
            else {
                throw new Error(`Error: Seed phrase provided in environment variable ${envVar} is invalid.`);
            }
        }
        else if (interactive) {
            const promptResult = yield (0, prompts_1.default)([
                {
                    type: "password",
                    name: "seed",
                    message: `Specify a ${inputName} for the ${accountRole} account`,
                    validate: (input) => validateInput(input),
                },
            ]);
            // If SIGTERM is issued while prompting, it will log a bogus address anyways and exit without error.
            // To avoid this, we check if prompt was successful, before returning.
            if (promptResult.seed) {
                return generateKeyring(promptResult.seed);
            }
        }
        throw new Error(`Error: Could not retrieve ${inputName}`);
    });
}
exports.initKeyringFromEnvOrPrompt = initKeyringFromEnvOrPrompt;
function validateECDSAKey(pk) {
    const keyring = initECDSAKeyringPairFromPK(pk);
    const msg = "";
    const sig = keyring.sign(msg);
    return keyring.verify(msg, sig, keyring.publicKey);
}
exports.validateECDSAKey = validateECDSAKey;
function getStringFromEnvVar(envVar) {
    if (envVar === undefined) {
        throw new Error("Error: Unexpected type; could not retrieve seed phrase or PK from environment variable.");
    }
    return envVar;
}
