import { InvalidArgumentError, Option } from 'commander';
import { isAddress, parseUnits } from 'ethers';
import { validateAddress } from '@polkadot/util-crypto/address';
import { BN } from '..';

// Most used options are URL, NO INPUT, JSON, eCDSA, aDDRESS, AMOUNT, TO, EMV-ADDRESS
// Create consts for each one using the Option class and export them to be used by other commands

// Connection
export const urlOption = new Option('-u, --url [url]', 'URL of the node to connect to').default('ws://127.0.0.1:9944');

// Addresses
export const evmAddressOption = new Option('--evm-address [address]', 'Specify EVM address').argParser(parseEVMAddress);
export const substrateAddressOption = new Option(
    '--substrate-address [address]',
    'Specify Substrate address',
).argParser(parseSubstrateAddress);
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
