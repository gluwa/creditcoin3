import { InvalidArgumentError, Option } from 'commander';
import { isAddress } from 'ethers';
import { validateAddress } from '@polkadot/util-crypto/address';

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
function parseEVMAddress(value: string, previous: string): string {
    if (isAddress(value)) {
        return value;
    } else {
        throw new InvalidArgumentError('Not a valid EVM address.');
    }
}
function parseSubstrateAddress(value: string, previous: string): string {
    try {
        validateAddress(value);
    } catch (e: any) {
        throw new InvalidArgumentError('Not a valid Substrate address.');
    }
    return value;
}

// Amounts
export const amountOption = new Option('--amount [amount]', 'CTC amount');

// I/O
export const jsonOption = new Option('--json', 'Output as JSON');
export const noInputOption = new Option('--no-input', 'Do not prompt for input');

// Crypto
export const ecdsaOption = new Option('--ecdsa', 'Use ECDSA signature instead of mnemonic');
