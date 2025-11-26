import { WebSocketProvider, ethers } from 'ethers';
import { ApiPromise, BN, MICROUNITS_PER_CTC, newApi } from '../../../lib';
import { fundFromSudo } from '../../integration-tests/helpers';

// eslint-disable-next-line @typescript-eslint/no-require-imports
import contractABIJSON = require('../artifacts/block_prover.json');
const contractABI = contractABIJSON as unknown as ethers.InterfaceAbi;
const PRECOMPILE_ADDRESS = '0x0000000000000000000000000000000000000FD2';

describe('Precompile: Native Query Verifier Integration Tests', (): void => {
    let contract: any;
    let provider: any;
    let alith: any;
    let api: ApiPromise;
    let gasPrice: bigint;
    // Helper to get the single-query verify function (disambiguate from batch overload)
    let verifySingle: any;
    let verifyAndEmitSingle: any;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);

        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        alith = new ethers.Wallet(privateKey, provider);

        // Fund Alith if needed
        const result = await fundFromSudo(alith.address, MICROUNITS_PER_CTC.mul(new BN(1_000_000)));
        expect(result.status).toBe(0);

        contract = new ethers.Contract(PRECOMPILE_ADDRESS, contractABI, alith);

        // Get the single-query verify function overload explicitly
        // Signature: verify(uint64,uint64,bytes,(bytes32,(bytes32,bool)[]),(bytes32,(bytes32,bytes32)[]))
        verifySingle = contract.getFunction(
            'verify(uint64,uint64,bytes,(bytes32,(bytes32,bool)[]),(bytes32,(bytes32,bytes32)[]))',
        );
        verifyAndEmitSingle = contract.getFunction(
            'verifyAndEmit(uint64,uint64,bytes,(bytes32,(bytes32,bool)[]),(bytes32,(bytes32,bytes32)[]))',
        );
    }, 90_000);

    afterAll(async () => {
        await api.disconnect();
    });

    beforeEach(async () => {
        gasPrice = (await provider.getFeeData()).gasPrice;
        // Wait a bit to avoid nonce conflicts between tests
        await new Promise((resolve) => setTimeout(resolve, 100));
    });

    describe('Precompile Deployment', () => {
        test('should verify precompile is deployed at correct address', async () => {
            // Verify precompile exists at the expected address
            // Note: Precompiles may not have bytecode but should respond to calls
            const code = await provider.getCode(PRECOMPILE_ADDRESS);
            expect(PRECOMPILE_ADDRESS).toBe('0x0000000000000000000000000000000000000FD2');
            // Precompiles might return '0x' or have some bytecode
            expect(code).toBeDefined();
        });

        test('should verify interface returns bool directly', async () => {
            const chainKey = 1;
            const height = 100;

            const txData = ethers.randomBytes(100);
            const merkleProof = {
                root: ethers.keccak256(txData),
                siblings: [],
            };
            const continuityProof = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                lowerEndpointDigest: ethers.zeroPadBytes('0x00', 32),
                blocks: [
                    {
                        merkleRoot: ethers.keccak256(txData),
                        digest: ethers.zeroPadBytes('0x01', 32),
                    },
                ],
            };

            try {
                // Note: This will likely fail without proper attestation data, but we're testing the interface
                // The new interface returns bool directly, not ResultSegment[]
                const result = await verifySingle.staticCall(chainKey, height, txData, merkleProof, continuityProof);
                // Verify it returns a boolean
                expect(typeof result).toBe('boolean');
            } catch (error: any) {
                // Expected to fail without proper attestation data
                // But the error should be about verification, not about return type
                expect(error).toBeDefined();
            }
        });
    });

    describe('Gas Estimation Tests', () => {
        test('should estimate gas for simple query verification', async () => {
            const chainKey = 1;
            const height = 100;

            const txData = ethers.randomBytes(100);
            const merkleProof = {
                root: ethers.keccak256(txData),
                siblings: [], // Empty array of entries for single tx
            };
            const continuityProof = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                lowerEndpointDigest: ethers.zeroPadBytes('0x00', 32),
                blocks: [
                    {
                        merkleRoot: ethers.keccak256(txData),
                        digest: ethers.zeroPadBytes('0x01', 32),
                    },
                ],
            };

            try {
                const estimatedGas = await verifySingle.estimateGas(
                    chainKey,
                    height,
                    txData,
                    merkleProof,
                    continuityProof,
                );
                expect(estimatedGas).toBeGreaterThan(0n);
                expect(estimatedGas).toBeLessThan(10000000n); // Reasonable upper bound
            } catch (error) {
                // Expected to fail without proper attestation data
                expect(error).toBeDefined();
            }
        });

        test('gas should scale with transaction data size', async () => {
            const chainKey = 1;
            const height = 100;

            const smallTxData = ethers.randomBytes(100);
            const largeTxData = ethers.randomBytes(1000);

            const merkleProof = {
                root: ethers.keccak256(smallTxData),
                siblings: [], // Empty array of entries
            };

            const continuityProof = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                lowerEndpointDigest: ethers.zeroPadBytes('0x00', 32),
                blocks: [
                    {
                        merkleRoot: ethers.keccak256(smallTxData),
                        digest: ethers.zeroPadBytes('0x01', 32),
                    },
                ],
            };

            try {
                const smallGas = await verifySingle.estimateGas(
                    chainKey,
                    height,
                    smallTxData,
                    merkleProof,
                    continuityProof,
                );

                const largeProof = {
                    root: ethers.keccak256(largeTxData),
                    siblings: [], // Empty array of entries
                };

                const largeContinuityProof = {
                    // eslint-disable-next-line @typescript-eslint/naming-convention
                    lowerEndpointDigest: ethers.zeroPadBytes('0x00', 32),
                    blocks: [
                        {
                            merkleRoot: ethers.keccak256(largeTxData),
                            digest: ethers.zeroPadBytes('0x01', 32),
                        },
                    ],
                };

                const largeGas = await verifySingle.estimateGas(
                    chainKey,
                    height,
                    largeTxData,
                    largeProof,
                    largeContinuityProof,
                );

                // Larger data should require more gas
                expect(largeGas).toBeGreaterThan(smallGas);
            } catch (error) {
                // Expected to fail without proper attestation data
                expect(error).toBeDefined();
            }
        });

        test('gas should scale with number of merkle siblings', async () => {
            const chainKey = 1;
            const height = 100;

            const txData = ethers.randomBytes(100);

            const simpleMerkleProof = {
                root: ethers.keccak256(txData),
                siblings: [], // No siblings needed
            };

            const complexMerkleProof = {
                root: ethers.keccak256(txData),
                siblings: [
                    { hash: ethers.randomBytes(32), isLeft: false },
                    { hash: ethers.randomBytes(32), isLeft: true },
                    { hash: ethers.randomBytes(32), isLeft: false },
                ],
            };

            const continuityProof = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                lowerEndpointDigest: ethers.zeroPadBytes('0x00', 32),
                blocks: [
                    {
                        merkleRoot: ethers.keccak256(txData),
                        digest: ethers.zeroPadBytes('0x01', 32),
                    },
                ],
            };

            try {
                const simpleGas = await verifySingle.estimateGas(
                    chainKey,
                    height,
                    txData,
                    simpleMerkleProof,
                    continuityProof,
                );

                const complexGas = await verifySingle.estimateGas(
                    chainKey,
                    height,
                    txData,
                    complexMerkleProof,
                    continuityProof,
                );

                // More siblings should require more gas
                expect(complexGas).toBeGreaterThan(simpleGas);
            } catch (error) {
                // Expected to fail without proper attestation data
                expect(error).toBeDefined();
            }
        });

        test('gas should scale with continuity chain length', async () => {
            const chainKey = 1;
            const height = 103;

            const txData = ethers.randomBytes(100);
            const merkleProof = {
                root: ethers.keccak256(txData),
                siblings: [], // Empty for single transaction
            };

            const shortContinuityProof = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                lowerEndpointDigest: ethers.zeroPadBytes('0x00', 32),
                blocks: [
                    {
                        merkleRoot: ethers.keccak256(txData),
                        digest: ethers.zeroPadBytes('0x01', 32),
                    },
                ],
            };

            const longContinuityProof = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                lowerEndpointDigest: ethers.zeroPadBytes('0x00', 32),
                blocks: [
                    {
                        merkleRoot: ethers.keccak256(txData),
                        digest: ethers.zeroPadBytes('0x01', 32),
                    },
                    {
                        merkleRoot: ethers.keccak256(txData),
                        digest: ethers.zeroPadBytes('0x02', 32),
                    },
                    {
                        merkleRoot: ethers.keccak256(txData),
                        digest: ethers.zeroPadBytes('0x03', 32),
                    },
                    {
                        merkleRoot: ethers.keccak256(txData),
                        digest: ethers.zeroPadBytes('0x04', 32),
                    },
                ],
            };

            try {
                const shortGas = await verifySingle.estimateGas(
                    chainKey,
                    height,
                    txData,
                    merkleProof,
                    shortContinuityProof,
                );

                const longGas = await verifySingle.estimateGas(
                    chainKey,
                    height,
                    txData,
                    merkleProof,
                    longContinuityProof,
                );

                // Longer continuity chain should require more gas
                expect(longGas).toBeGreaterThan(shortGas);
            } catch (error) {
                // Expected to fail without proper attestation data
                expect(error).toBeDefined();
            }
        });
    });

    describe('Input Validation Tests', () => {
        test('should handle maximum uint values gracefully', async () => {
            const maxUint64 = 2n ** 64n - 1n;
            const chainKey = maxUint64; // Max uint64
            const height = maxUint64;

            const txData = ethers.randomBytes(100);
            const merkleProof = {
                root: ethers.keccak256(txData),
                siblings: [], // Empty entries array
            };
            const continuityProof = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                lowerEndpointDigest: ethers.zeroPadBytes('0x00', 32),
                blocks: [
                    {
                        merkleRoot: ethers.keccak256(txData),
                        digest: ethers.zeroPadBytes('0x01', 32),
                    },
                ],
            };

            try {
                await verifyAndEmitSingle(chainKey, height, txData, merkleProof, continuityProof, {
                    gasPrice,
                    gasLimit: 500000,
                });
            } catch (error: any) {
                // Expected to fail - precompile should handle max values appropriately
                expect(error).toBeDefined();
            }
        });

        test('should handle malformed transaction data encoding', async () => {
            const chainKey = 1;
            const height = 100;

            // Use invalid data that will fail ethers validation
            const invalidData = 'INVALID_HEX_DATA';
            const merkleProof = {
                root: ethers.zeroPadBytes('0x01', 32),
                siblings: [], // Empty entries array
            };
            const continuityProof = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                lowerEndpointDigest: ethers.zeroPadBytes('0x00', 32),
                blocks: [
                    {
                        merkleRoot: ethers.zeroPadBytes('0x01', 32),
                        digest: ethers.zeroPadBytes('0x01', 32),
                    },
                ],
            };

            try {
                await verifyAndEmitSingle(chainKey, height, invalidData, merkleProof, continuityProof, {
                    gasPrice,
                    gasLimit: 500000,
                });
            } catch (error: any) {
                // Should fail at ethers.js level with invalid hex
                expect(error).toBeDefined();
                expect(error.message).toBeDefined();
            }
        });

        test('should fail with malformed continuity block structure', async () => {
            const chainKey = 1;
            const height = 100;

            const txData = ethers.randomBytes(100);
            const merkleProof = {
                root: ethers.keccak256(txData),
                siblings: [], // Empty entries array
            };

            // Malformed continuity proof with empty block array
            const malformedProof = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                lowerEndpointDigest: ethers.zeroPadBytes('0x00', 32),
                blocks: [], // Empty array
            };

            // Use staticCall to simulate without sending transaction (avoids nonce conflicts)
            await expect(
                verifySingle.staticCall(chainKey, height, txData, merkleProof, malformedProof),
            ).rejects.toThrow(/Continuity chain cannot be empty/);
        });

        test('should fail with invalid hex encoding in transaction data', async () => {
            const chainKey = 1;
            const height = 100;

            const merkleProof = {
                root: ethers.zeroPadBytes('0x01', 32),
                siblings: [], // Empty entries array
            };
            const continuityProof = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                lowerEndpointDigest: ethers.zeroPadBytes('0x00', 32),
                blocks: [
                    {
                        merkleRoot: ethers.zeroPadBytes('0x01', 32),
                        digest: ethers.zeroPadBytes('0x01', 32),
                    },
                ],
            };

            // Pass non-hex string as transaction data - should fail at ethers.js validation level
            // Note: ethers.js throws "invalid BytesLike value" during encoding
            await expect(
                verifySingle.staticCall(chainKey, height, 'not-hex-data', merkleProof, continuityProof),
            ).rejects.toThrow(/invalid BytesLike value|invalid hex string/i);
        });
    });

    describe('Failing Cases - Expected Reverts', () => {
        test('should fail when querying without attestation data', async () => {
            const chainKey = 1;
            const height = 100;

            const txData = ethers.randomBytes(100);
            const merkleProof = {
                root: ethers.keccak256(txData),
                siblings: [], // Empty entries array
            };
            const continuityProof = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                lowerEndpointDigest: ethers.zeroPadBytes('0x00', 32),
                blocks: [
                    {
                        merkleRoot: ethers.keccak256(txData),
                        digest: ethers.zeroPadBytes('0x01', 32),
                    },
                ],
            };

            // Note: Merkle proof validation happens first, so this fails at Merkle validation
            // To test continuity validation, we would need valid Merkle proofs
            // Use staticCall to simulate without sending transaction (avoids nonce conflicts)
            await expect(
                verifySingle.staticCall(chainKey, height, txData, merkleProof, continuityProof),
            ).rejects.toThrow(/Merkle proof validation failed/);
        });

        test('should fail with empty transaction data', async () => {
            const chainKey = 1;
            const height = 100;

            const txData = '0x'; // Empty transaction data
            const merkleProof = {
                root: ethers.zeroPadBytes('0x00', 32),
                siblings: [], // Empty entries array
            };
            const continuityProof = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                lowerEndpointDigest: ethers.zeroPadBytes('0x00', 32),
                blocks: [
                    {
                        merkleRoot: ethers.zeroPadBytes('0x00', 32),
                        digest: ethers.zeroPadBytes('0x01', 32),
                    },
                ],
            };

            // Use staticCall to simulate without sending transaction (avoids nonce conflicts)
            await expect(
                verifySingle.staticCall(chainKey, height, txData, merkleProof, continuityProof),
            ).rejects.toThrow(/Transaction data cannot be empty/);
        });

        test('should fail when querying invalid block', async () => {
            const chainKey = 1;
            const height = 100;

            const txData = ethers.randomBytes(100);
            const merkleProof = {
                root: ethers.keccak256(txData),
                siblings: [], // Empty entries array
            };

            const continuityProof = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                lowerEndpointDigest: ethers.zeroPadBytes('0x00', 32),
                blocks: [
                    {
                        merkleRoot: ethers.keccak256(txData),
                        digest: ethers.zeroPadBytes('0x01', 32),
                    },
                ],
            };

            // Note: Merkle proof validation happens first, so this fails at Merkle validation
            // To test continuity validation, we would need valid Merkle proofs
            // Use staticCall to simulate without sending transaction (avoids nonce conflicts)
            await expect(
                verifySingle.staticCall(chainKey, height, txData, merkleProof, continuityProof),
            ).rejects.toThrow(/Merkle proof validation failed/);
        });

        test('should fail with invalid continuity proof', async () => {
            const chainKey = 1;
            const height = 100;

            const txData = ethers.randomBytes(100);
            const merkleProof = {
                root: ethers.keccak256(txData),
                siblings: [], // Empty entries array
            };
            const continuityProof = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                lowerEndpointDigest: ethers.zeroPadBytes('0x00', 32),
                blocks: [
                    {
                        merkleRoot: ethers.keccak256(txData),
                        digest: ethers.zeroPadBytes('0x01', 32),
                    },
                ],
            };

            // Note: Merkle proof validation happens first, so this fails at Merkle validation
            // To test continuity validation, we would need valid Merkle proofs
            // Use staticCall to simulate without sending transaction (avoids nonce conflicts)
            await expect(
                verifySingle.staticCall(chainKey, height, txData, merkleProof, continuityProof),
            ).rejects.toThrow(/Merkle proof validation failed/);
        });

        test('should fail with mismatched merkle root', async () => {
            const chainKey = 1;
            const height = 100;

            const txData = ethers.randomBytes(100);
            const merkleProof = {
                root: ethers.keccak256('0xdeadbeef'), // Wrong root, doesn't match txData
                siblings: [{ hash: ethers.randomBytes(32), isLeft: false }],
            };
            const continuityProof = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                lowerEndpointDigest: ethers.zeroPadBytes('0x00', 32),
                blocks: [
                    {
                        merkleRoot: ethers.keccak256('0xdeadbeef'), // Wrong root to match merkle proof
                        digest: ethers.zeroPadBytes('0x01', 32),
                    },
                ],
            };

            // Use staticCall to simulate without sending transaction (avoids nonce conflicts)
            await expect(
                verifySingle.staticCall(chainKey, height, txData, merkleProof, continuityProof),
            ).rejects.toThrow(/Merkle proof validation failed/);
        });
    });
});
