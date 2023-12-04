import { OptionValues } from 'commander';
import { HDNodeWallet, Wallet, Mnemonic } from 'ethers';
import { parseBoolean } from '../parsing';
import { getStringFromEnvVar } from '../account/keyring';
import prompts from 'prompts';
import { getErrorMessage } from '../error';

export async function initEVMCallerWallet(options: OptionValues): Promise<Wallet | HDNodeWallet> {
    try {
        return await initEthWalletFromEnvOrPrompt('EVM_SECRET', options);
    } catch (e) {
        console.error(getErrorMessage(e));
        process.exit(1);
    }
}

function initEthWalletFromPK(pk: string) {
    const wallet = new Wallet(pk);
    return wallet;
}

function initEthWalletFromMnemonic(mnemonic: string) {
    const wallet = Wallet.fromPhrase(mnemonic);
    return wallet;
}

export async function initEthWalletFromEnvOrPrompt(
    envVar: string,
    options: OptionValues,
): Promise<Wallet | HDNodeWallet> {
    // General configs
    const interactive = parseBoolean(options.input);
    const inputName = options.useEcdsa ? 'private key' : 'seed phrase';
    console.log(options.useEcdsa);
    const validateInput = options.useEcdsa ? () => true : (input: string) => Mnemonic.isValidMnemonic(input);
    const generateKeyring = options.useEcdsa ? initEthWalletFromPK : initEthWalletFromMnemonic;

    if (!interactive && !process.env[envVar]) {
        throw new Error(
            `Error: Must specify a ${inputName} for the EVM account in the environment variable ${envVar} or use an interactive shell.`,
        );
    }

    if (typeof process.env[envVar] === 'string') {
        const input = getStringFromEnvVar(process.env[envVar]);
        if (validateInput(input)) {
            return generateKeyring(input);
        } else {
            throw new Error(`Error: Seed phrase provided in environment variable ${envVar} is invalid.`);
        }
    } else if (interactive) {
        const promptResult = await prompts([
            {
                type: 'password',
                name: 'seed',
                message: `Specify a ${inputName} for the EVM account`,
                validate: (input) => validateInput(input as string),
            },
        ]);
        // If SIGTERM is issued while prompting, it will log a bogus address anyways and exit without error.
        // To avoid this, we check if prompt was successful, before returning.
        if (promptResult.seed) {
            return generateKeyring(promptResult.seed as string);
        }
    }
    throw new Error(`Error: Could not retrieve ${inputName}`);
}
