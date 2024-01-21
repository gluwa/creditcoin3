import { InvalidArgumentError, Option } from 'commander';
import { isAddress, parseUnits } from 'ethers';
import { validateAddress } from '@polkadot/util-crypto/address';
import { BN } from '..';

// Most used options are URL, NO INPUT, JSON, eCDSA, aDDRESS, AMOUNT, TO, EMV-ADDRESS
// Create consts for each one using the Option class and export them to be used by other commands

// Connection
export const urlOption = new Option('-u, --url [url]', 'URL of the node to connect to').default('ws://127.0.0.1:9944');

// Addresses
export interface ValidatedAddress {
    address: string;
    type: 'Substrate' | 'EVM';
}

export const evmAddressOption = new Option('--evm-address [address]', 'Specify EVM address').argParser(parseEVMAddress);
export const substrateAddressOption = new Option(
    '--substrate-address [address]',
    'Specify Substrate address',
).argParser(parseSubstrateAddress);
export const unknownAddressOption = new Option('--address [address]', 'Specify address').argParser(parseAddress);
// Address parsing
export function parseEVMAddress(value: string): string {
    if (isAddress(value)) {
        return value;
    } else {
        throw new InvalidArgumentError('Not a valid EVM address.');
    }
}
export function parseSubstrateAddress(value: string): string {
    try {
        validateAddress(value);
    } catch (e: any) {
        throw new InvalidArgumentError('Not a valid Substrate address.');
    }
    return value;
}

export function parseAddress(value: string): ValidatedAddress {
    // Parsed has to be one of EVM or Substrate addresses
    try {
        return {
            address: parseEVMAddress(value),
            type: 'EVM',
        };
    } catch {
        try {
            return {
                address: parseSubstrateAddress(value),
                type: 'Substrate',
            };
        } catch {
            throw new InvalidArgumentError('Not a valid Substrate or EVM address.');
        }
    }
}

// Amounts
export const amountOption = new Option('--amount [amount]', 'CTC amount').argParser(parseAmount);
// Amount parsing
export function parseAmount(value: string): BN {
    try {
        const parsed = positiveBigNumberFromString(value);
        return new BN(parsed.toString());
    } catch (e: any) {
        throw new InvalidArgumentError('Not a valid amount.');
    }
}
function positiveBigNumberFromString(amount: any) {
    const parsedValue = parseUnits(amount as string, 18);

    if (parsedValue === BigInt(0)) {
        throw new Error('Must be greater than 0');
    }

    if (parsedValue < BigInt(0)) {
        throw new Error('Must be a positive number');
    }

    return parsedValue;
}

// Session
export const eraOption = new Option('--era [era]', 'Specify era to distribute rewards for').argParser(parseEra);

// Era parsing
export function parseEra(value: string): number {
    // Only positive integers are allowed
    const parsedEra = parseInt(value, 10);

    if (isNaN(parsedEra)) {
        throw new InvalidArgumentError('Not a valid era.');
    }

    if (parsedEra < 0) {
        throw new InvalidArgumentError('Era must be a positive integer.');
    }

    return parsedEra;
}

// I/O
export const jsonOption = new Option('--json', 'Output as JSON');
export const noInputOption = new Option('--no-input', 'Do not prompt for input');

// Crypto
export const ecdsaOption = new Option('--ecdsa', 'Use ECDSA signature instead of mnemonic');

export const proxyOption = new Option('-p, --proxy <proxy addr>', 'The proxy address to use for this call').argParser(
    parseProxy,
);
export function parseProxy(value: string): string {
    try {
        validateAddress(value);
    } catch (e: any) {
        throw new InvalidArgumentError('Not a valid Substrate address.');
    }
    return value;
}

export const proxyTypeOption = new Option('--type [type]', 'The type of proxy');

export const delayOption = new Option('--delay [delay]', 'The delay for the proxy').argParser(parseProxyDelay);
export function parseProxyDelay(value: string): number {
    const parsedValue = parseInt(value, 10);
    if (isNaN(parsedValue)) {
        throw new Error(`ERROR: Could not parse delay: ${value}`);
    }
    return parsedValue;
}
