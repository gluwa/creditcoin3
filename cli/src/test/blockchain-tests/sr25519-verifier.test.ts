import { WebSocketProvider, ethers } from 'ethers';
import { Keyring } from '@polkadot/keyring';
import { mnemonicGenerate, cryptoWaitReady } from '@polkadot/util-crypto';
import { u8aToHex } from '@polkadot/util';
import { newApi, ApiPromise, BN, MICROUNITS_PER_CTC } from '../../lib';
import { fundFromSudo } from '../integration-tests/helpers';

// eslint-disable-next-line @typescript-eslint/no-require-imports
import contractABIJSON = require('./artifacts/sr25519_verifier.json');

const contractABI = contractABIJSON.contracts['sol/sr25519_verifier.sol:Sr25519Verifier'].abi;

describe('Precompile: Sr25519Verifier.verify()', (): void => {
    let contract: any;
    let provider: any;
    let alith: any;
    let api: ApiPromise;
    let gasPrice: bigint;
    let gasLimit: number;
    let keyring: Keyring;

    beforeAll(async () => {
        // Wait for crypto to be ready
        await cryptoWaitReady();

        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);

        // Precompile contract deployed at 5049 (0x13B9) in hex, see runtime/src/precompiles.rs
        const precompileContractAddress = '0x00000000000000000000000000000000000013B9';

        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        alith = new ethers.Wallet(privateKey, provider);

        // Fund the account for gas fees
        const result = await fundFromSudo(alith.address, MICROUNITS_PER_CTC.mul(new BN(2_000_000)));
        expect(result.status).toBe(0);

        contract = new ethers.Contract(precompileContractAddress, contractABI, alith);

        // Initialize keyring for sr25519 signatures
        keyring = new Keyring({ type: 'sr25519' });

        gasLimit = 10000000;
    }, 90_000);

    afterAll(async () => {
        await api.disconnect();
    });

    beforeEach(async () => {
        gasPrice = (await provider.getFeeData()).gasPrice;
    });

    describe('Happy paths', () => {
        test('should return true for valid signature', async () => {
            // Generate a keypair
            const pair = keyring.addFromMnemonic(mnemonicGenerate());
            const message = 'Hello, Creditcoin!';
            const messageBytes = new TextEncoder().encode(message);

            // Sign the message
            const signature = pair.sign(messageBytes);

            // Convert to hex strings
            const messageHex = u8aToHex(messageBytes);
            const signatureHex = u8aToHex(signature);
            const publicKeyHex = u8aToHex(pair.publicKey);

            // Call the precompile
            const result = await contract.verify(messageHex, signatureHex, publicKeyHex, {
                gasPrice,
                gasLimit,
            });

            expect(result).toBe(true);
        });

        test('should return true for empty message with valid signature', async () => {
            // Generate a keypair
            const pair = keyring.addFromMnemonic(mnemonicGenerate());
            const messageBytes = new Uint8Array(0);

            // Sign the empty message
            const signature = pair.sign(messageBytes);

            // Convert to hex strings
            const messageHex = u8aToHex(messageBytes);
            const signatureHex = u8aToHex(signature);
            const publicKeyHex = u8aToHex(pair.publicKey);

            // Call the precompile
            const result = await contract.verify(messageHex, signatureHex, publicKeyHex, {
                gasPrice,
                gasLimit,
            });

            expect(result).toBe(true);
        });

        test('should return true for long message with valid signature', async () => {
            // Generate a keypair
            const pair = keyring.addFromMnemonic(mnemonicGenerate());

            const longMessage = 'A'.repeat(1048576);
            const messageBytes = new TextEncoder().encode(longMessage);

            // Sign the long message
            const signature = pair.sign(messageBytes);

            // Convert to hex strings
            const messageHex = u8aToHex(messageBytes);
            const signatureHex = u8aToHex(signature);
            const publicKeyHex = u8aToHex(pair.publicKey);

            // Call the precompile
            const result = await contract.verify(messageHex, signatureHex, publicKeyHex, {
                gasPrice,
                gasLimit: 20000000, // Higher gas limit for longer message
            });

            expect(result).toBe(true);
        });

        test('should return true for binary data with valid signature', async () => {
            // Generate a keypair
            const pair = keyring.addFromMnemonic(mnemonicGenerate());

            // Create binary data
            const messageBytes = new Uint8Array([0x00, 0x01, 0x02, 0x03, 0xff, 0xfe, 0xfd, 0xfc]);

            // Sign the binary data
            const signature = pair.sign(messageBytes);

            // Convert to hex strings
            const messageHex = u8aToHex(messageBytes);
            const signatureHex = u8aToHex(signature);
            const publicKeyHex = u8aToHex(pair.publicKey);

            // Call the precompile
            const result = await contract.verify(messageHex, signatureHex, publicKeyHex, {
                gasPrice,
                gasLimit,
            });

            expect(result).toBe(true);
        });
    });

    describe('Unhappy paths', () => {
        test('should return false for tampered message', async () => {
            // Generate a keypair
            const pair = keyring.addFromMnemonic(mnemonicGenerate());
            const originalMessage = 'Hello, World!';
            const originalMessageBytes = new TextEncoder().encode(originalMessage);

            // Sign the original message
            const signature = pair.sign(originalMessageBytes);

            // Tamper with the message
            const tamperedMessage = 'Hello, World 1!';
            const tamperedMessageBytes = new TextEncoder().encode(tamperedMessage);

            // Convert to hex strings
            const tamperedMessageHex = u8aToHex(tamperedMessageBytes);
            const signatureHex = u8aToHex(signature);
            const publicKeyHex = u8aToHex(pair.publicKey);

            // Call the precompile with tampered message
            const result = await contract.verify(tamperedMessageHex, signatureHex, publicKeyHex, {
                gasPrice,
                gasLimit,
            });

            expect(result).toBe(false);
        });

        test('should return false for wrong public key', async () => {
            // Generate two keypairs
            const pair1 = keyring.addFromMnemonic(mnemonicGenerate());
            const pair2 = keyring.addFromMnemonic(mnemonicGenerate());

            const message = 'Hello, World!';
            const messageBytes = new TextEncoder().encode(message);

            // Sign with pair1
            const signature = pair1.sign(messageBytes);

            // Convert to hex strings using pair2's public key
            const messageHex = u8aToHex(messageBytes);
            const signatureHex = u8aToHex(signature);
            const wrongPublicKeyHex = u8aToHex(pair2.publicKey);

            // Call the precompile with wrong public key
            const result = await contract.verify(messageHex, signatureHex, wrongPublicKeyHex, {
                gasPrice,
                gasLimit,
            });

            expect(result).toBe(false);
        });

        test('should return false for tampered signature', async () => {
            // Generate a keypair
            const pair = keyring.addFromMnemonic(mnemonicGenerate());
            const message = 'Hello, World!';
            const messageBytes = new TextEncoder().encode(message);

            // Sign the message
            const signature = pair.sign(messageBytes);

            // Tamper with the signature (flip a bit)
            const tamperedSignature = new Uint8Array(signature);
            tamperedSignature[0] = 0x01;

            // Convert to hex strings
            const messageHex = u8aToHex(messageBytes);
            const tamperedSignatureHex = u8aToHex(tamperedSignature);
            const publicKeyHex = u8aToHex(pair.publicKey);

            // Call the precompile with tampered signature
            const result = await contract.verify(messageHex, tamperedSignatureHex, publicKeyHex, {
                gasPrice,
                gasLimit,
            });

            expect(result).toBe(false);
        });

        test('should revert for signature that is too short', async () => {
            // Generate a keypair
            const pair = keyring.addFromMnemonic(mnemonicGenerate());
            const message = 'Hello, World!';
            const messageBytes = new TextEncoder().encode(message);

            // Create an invalid signature (32 bytes instead of 64)
            const invalidSignature = new Uint8Array(32);

            // Convert to hex strings
            const messageHex = u8aToHex(messageBytes);
            const invalidSignatureHex = u8aToHex(invalidSignature);
            const publicKeyHex = u8aToHex(pair.publicKey);

            // Call the precompile with invalid signature length
            await expect(
                contract.verify(messageHex, invalidSignatureHex, publicKeyHex, {
                    gasPrice,
                    gasLimit,
                }),
            ).rejects.toThrow(/Invalid signature length: must be exactly 64 bytes/);
        });

        test('should revert for signature that is too long', async () => {
            // Generate a keypair
            const pair = keyring.addFromMnemonic(mnemonicGenerate());
            const message = 'Hello, World!';
            const messageBytes = new TextEncoder().encode(message);

            // Create an invalid signature (100 bytes instead of 64)
            const invalidSignature = new Uint8Array(100);

            // Convert to hex strings
            const messageHex = u8aToHex(messageBytes);
            const invalidSignatureHex = u8aToHex(invalidSignature);
            const publicKeyHex = u8aToHex(pair.publicKey);

            // Call the precompile with invalid signature length
            await expect(
                contract.verify(messageHex, invalidSignatureHex, publicKeyHex, {
                    gasPrice,
                    gasLimit,
                }),
            ).rejects.toThrow(/Value is too large for length/);
        });

        test('should revert for message exceeding 3MB limit', async () => {
            // Generate a keypair
            const pair = keyring.addFromMnemonic(mnemonicGenerate());

            // Create a message larger than 3MB (3MB + 1 byte)
            const largeMessage = 'A'.repeat(3145729);
            const messageBytes = new TextEncoder().encode(largeMessage);

            // Sign the message (this will work in substrate)
            const signature = pair.sign(messageBytes);

            // Convert to hex strings
            const messageHex = u8aToHex(messageBytes);
            const signatureHex = u8aToHex(signature);
            const publicKeyHex = u8aToHex(pair.publicKey);

            // Call the precompile - should revert due to size limit
            await expect(
                contract.verify(messageHex, signatureHex, publicKeyHex, {
                    gasPrice,
                    gasLimit: 75_000_000,
                }),
            ).rejects.toThrow(/Value is too large for length/);
        }, 120_000);
    });
});
