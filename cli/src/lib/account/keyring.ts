import { mnemonicValidate } from '@polkadot/util-crypto';
import { Keyring, KeyringPair } from '..';
import prompts from 'prompts';
import { getErrorMessage } from '../error';
import { OptionValues } from 'commander';
import { parseBoolean } from '../parsing';

export function initKeyringPair(seed: string) {
    const keyring = new Keyring({ type: 'sr25519' });
    const pair = keyring.addFromUri(`${seed}`);
    return pair;
}
export function initECDSAKeyringPairFromPK(pk: string) {
    const keyring = new Keyring({ type: 'ecdsa' });
    const pair = keyring.addFromUri(`${pk}`);
    return pair;
}

export function initEthKeyringPair(seed: string, accIndex = 0) {
    const keyring = new Keyring({ type: 'ethereum' });
    const pair = keyring.addFromUri(`${seed}/m/44'/60'/0'/0/${accIndex}`);
    return pair;
}

export async function initCallerKeyring(options: OptionValues): Promise<KeyringPair> {
    try {
        return await initKeyringFromEnvOrPrompt('CC_SECRET', 'caller', options);
    } catch (e) {
        console.error(getErrorMessage(e));
        process.exit(1);
    }
}

export async function initProxyKeyring(options: OptionValues): Promise<KeyringPair | null> {
    if (!options.proxy) {
        return null;
    }

    try {
        return await initKeyringFromEnvOrPrompt('CC_PROXY_SECRET', 'proxy', options);
    } catch (e) {
        console.error(getErrorMessage(e));
        process.exit(1);
    }
}

export async function initKeyringFromEnvOrPrompt(
    envVar: string,
    accountRole: string,
    options: OptionValues,
): Promise<KeyringPair> {
    // General configs
    const interactive = parseBoolean(options.input);
    const inputName = options.useEcdsa ? 'private key' : 'seed phrase';
    const validateInput = options.useEcdsa ? () => true : mnemonicValidate;
    const generateKeyring = options.useEcdsa ? initECDSAKeyringPairFromPK : initKeyringPair;

    if (!interactive && !process.env[envVar]) {
        throw new Error(
            `Error: Must specify a ${inputName} for the ${accountRole} account in the environment variable ${envVar} or use an interactive shell.`,
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
                message: `Specify a ${inputName} for the ${accountRole} account`,
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

export function getStringFromEnvVar(envVar: string | undefined): string {
    if (envVar === undefined) {
        throw new Error('Error: Unexpected type; could not retrieve seed phrase or PK from environment variable.');
    }
    return envVar;
}
