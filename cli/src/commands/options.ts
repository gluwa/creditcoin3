import { Option } from 'commander';

// Most used options are URL, NO INPUT, JSON, eCDSA, aDDRESS, AMOUNT, TO, EMV-ADDRESS
// Create consts for each one using the Option class and export them to be used by other commands

// Connection
export const urlOption = new Option('-u, --url [url]', 'URL of the node to connect to').default('ws://127.0.0.1:9944');

// Addresses
export const evmAddressOption = new Option('--evm-address [address]', 'Specify EVM address');
export const substrateAddressOption = new Option('--substrate-address [address]', 'Specify Substrate address');
export const recipientOption = new Option('--recipient [recipient]', 'Specify recipient address');

// Amounts
export const amountOption = new Option('--amount [amount]', 'CTC amount');

// I/O
export const jsonOption = new Option('--json', 'Output as JSON');
export const noInputOption = new Option('--no-input', 'Do not prompt for input');

// Crypto
export const ecdsaOption = new Option('--ecdsa', 'Use ECDSA signature instead of mnemonic');
