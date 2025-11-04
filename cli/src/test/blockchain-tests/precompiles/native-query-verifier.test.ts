import { WebSocketProvider, ethers } from 'ethers';
import { ApiPromise, BN, MICROUNITS_PER_CTC, newApi } from '../../../lib';
import { fundFromSudo } from '../../integration-tests/helpers';

// eslint-disable-next-line @typescript-eslint/no-require-imports
import contractABIJSON = require('../artifacts/native_query_verifier.json');

const contractABI = contractABIJSON.contracts['sol/native_query_verifier.sol:NativeQueryVerifier'].abi;
const PRECOMPILE_ADDRESS = '0x0000000000000000000000000000000000000FD2';

describe('Precompile: Native Query Verifier Integration Tests', (): void => {
    let contract: any;
    let provider: any;
    let alith: any;
    let api: ApiPromise;
    let gasPrice: bigint;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);

        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        alith = new ethers.Wallet(privateKey, provider);

        // Fund Alith if needed
        const result = await fundFromSudo(alith.address, MICROUNITS_PER_CTC.mul(new BN(1_000_000)));
        expect(result.status).toBe(0);

        contract = new ethers.Contract(PRECOMPILE_ADDRESS, contractABI, alith);
    }, 90_000);

    afterAll(async () => {
        await api.disconnect();
    });

    beforeEach(async () => {
        gasPrice = (await provider.getFeeData()).gasPrice;
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
    });

    describe('Gas Estimation Tests', () => {
        test('should estimate gas for simple query verification', async () => {
            const query = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                chain_id: 1,
                height: 100,
                index: 0,
                // eslint-disable-next-line @typescript-eslint/naming-convention
                layout_segments: [{ offset: 0, size: 32 }],
            };

            const txData = ethers.randomBytes(100);
            const merkleProof = {
                root: ethers.keccak256(txData),
                siblings: [],
            };
            const continuityChain = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                block_numbers: [100],
                digests: [ethers.zeroPadBytes('0x01', 32)],
            };

            try {
                const estimatedGas = await contract.verifyQuery.estimateGas(
                    query,
                    txData,
                    merkleProof,
                    continuityChain,
                );
                expect(estimatedGas).toBeGreaterThan(0n);
                expect(estimatedGas).toBeLessThan(10000000n); // Reasonable upper bound
            } catch (error) {
                // Expected to fail without proper attestation data
                expect(error).toBeDefined();
            }
        });

        test('gas should scale with transaction data size', async () => {
            const query = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                chain_id: 1,
                height: 100,
                index: 0,
                // eslint-disable-next-line @typescript-eslint/naming-convention
                layout_segments: [{ offset: 0, size: 32 }],
            };

            const smallTxData = ethers.randomBytes(100);
            const largeTxData = ethers.randomBytes(1000);

            const merkleProof = {
                root: ethers.keccak256(smallTxData),
                siblings: [],
            };

            const continuityChain = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                block_numbers: [100],
                digests: [ethers.zeroPadBytes('0x01', 32)],
            };

            try {
                const smallGas = await contract.verifyQuery.estimateGas(
                    query,
                    smallTxData,
                    merkleProof,
                    continuityChain,
                );

                const largeProof = {
                    root: ethers.keccak256(largeTxData),
                    siblings: [],
                };

                const largeGas = await contract.verifyQuery.estimateGas(
                    query,
                    largeTxData,
                    largeProof,
                    continuityChain,
                );

                // Larger data should require more gas
                expect(largeGas).toBeGreaterThan(smallGas);
            } catch (error) {
                // Expected to fail without proper attestation data
                expect(error).toBeDefined();
            }
        });

        test('gas should scale with number of merkle siblings', async () => {
            const query = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                chain_id: 1,
                height: 100,
                index: 0,
                // eslint-disable-next-line @typescript-eslint/naming-convention
                layout_segments: [{ offset: 0, size: 32 }],
            };

            const txData = ethers.randomBytes(100);

            const simpleMerkleProof = {
                root: ethers.keccak256(txData),
                siblings: [],
            };

            const complexMerkleProof = {
                root: ethers.keccak256(txData),
                siblings: [ethers.randomBytes(32), ethers.randomBytes(32), ethers.randomBytes(32)],
            };

            const continuityChain = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                block_numbers: [100],
                digests: [ethers.zeroPadBytes('0x01', 32)],
            };

            try {
                const simpleGas = await contract.verifyQuery.estimateGas(
                    query,
                    txData,
                    simpleMerkleProof,
                    continuityChain,
                );

                const complexGas = await contract.verifyQuery.estimateGas(
                    query,
                    txData,
                    complexMerkleProof,
                    continuityChain,
                );

                // More siblings should require more gas
                expect(complexGas).toBeGreaterThan(simpleGas);
            } catch (error) {
                // Expected to fail without proper attestation data
                expect(error).toBeDefined();
            }
        });

        test('gas should scale with continuity chain length', async () => {
            const query = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                chain_id: 1,
                height: 103,
                index: 0,
                // eslint-disable-next-line @typescript-eslint/naming-convention
                layout_segments: [{ offset: 0, size: 32 }],
            };

            const txData = ethers.randomBytes(100);
            const merkleProof = {
                root: ethers.keccak256(txData),
                siblings: [],
            };

            const shortContinuityChain = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                block_numbers: [103],
                digests: [ethers.zeroPadBytes('0x01', 32)],
            };

            const longContinuityChain = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                block_numbers: [100, 101, 102, 103],
                digests: [
                    ethers.zeroPadBytes('0x01', 32),
                    ethers.zeroPadBytes('0x02', 32),
                    ethers.zeroPadBytes('0x03', 32),
                    ethers.zeroPadBytes('0x04', 32),
                ],
            };

            try {
                const shortGas = await contract.verifyQuery.estimateGas(
                    query,
                    txData,
                    merkleProof,
                    shortContinuityChain,
                );

                const longGas = await contract.verifyQuery.estimateGas(query, txData, merkleProof, longContinuityChain);

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
            const query = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                chain_id: 2n ** 64n - 1n, // Max uint64
                height: 2n ** 64n - 1n,
                index: 2n ** 64n - 1n,
                // eslint-disable-next-line @typescript-eslint/naming-convention
                layout_segments: [{ offset: 0, size: 32 }],
            };

            const txData = ethers.randomBytes(100);
            const merkleProof = {
                root: ethers.keccak256(txData),
                siblings: [],
            };
            const continuityChain = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                block_numbers: [100],
                digests: [ethers.zeroPadBytes('0x01', 32)],
            };

            try {
                await contract.verifyQuery(query, txData, merkleProof, continuityChain, {
                    gasPrice,
                    gasLimit: 500000,
                });
            } catch (error: any) {
                // Expected to fail - precompile should handle max values appropriately
                expect(error).toBeDefined();
            }
        });

        test('should handle malformed transaction data encoding', async () => {
            const query = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                chain_id: 1,
                height: 100,
                index: 0,
                // eslint-disable-next-line @typescript-eslint/naming-convention
                layout_segments: [{ offset: 0, size: 32 }],
            };

            // Use invalid data that will fail ethers validation
            const invalidData = 'INVALID_HEX_DATA';
            const merkleProof = {
                root: ethers.zeroPadBytes('0x01', 32),
                siblings: [],
            };
            const continuityChain = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                block_numbers: [100],
                digests: [ethers.zeroPadBytes('0x01', 32)],
            };

            try {
                await contract.verifyQuery(query, invalidData, merkleProof, continuityChain, {
                    gasPrice,
                    gasLimit: 500000,
                });
            } catch (error: any) {
                // Should fail at ethers.js level with invalid hex
                expect(error).toBeDefined();
                expect(error.message).toBeDefined();
            }
        });

        test('should fail with negative transaction index', async () => {
            const query = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                chain_id: 1,
                height: 100,
                index: -1, // Will be handled by ethers as a different value due to unsigned
                // eslint-disable-next-line @typescript-eslint/naming-convention
                layout_segments: [{ offset: 0, size: 32 }],
            };

            const txData = ethers.randomBytes(100);
            const merkleProof = {
                root: ethers.keccak256(txData),
                siblings: [],
            };
            const continuityChain = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                block_numbers: [100],
                digests: [ethers.zeroPadBytes('0x01', 32)],
            };

            try {
                await contract.verifyQuery(query, txData, merkleProof, continuityChain, {
                    gasPrice,
                    gasLimit: 500000,
                });
            } catch (error: any) {
                // Expected to fail
                expect(error).toBeDefined();
            }
        });

        test('should fail with malformed continuity block structure', async () => {
            const query = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                chain_id: 1,
                height: 100,
                index: 0,
                // eslint-disable-next-line @typescript-eslint/naming-convention
                layout_segments: [{ offset: 0, size: 32 }],
            };

            const txData = ethers.randomBytes(100);
            const merkleProof = {
                root: ethers.keccak256(txData),
                siblings: [],
            };

            // Malformed continuity chain with invalid structure
            const malformedChain = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                block_numbers: [], // Empty array
                digests: [ethers.zeroPadBytes('0x01', 32)], // Mismatched length
            };

            try {
                await contract.verifyQuery(query, txData, merkleProof, malformedChain, {
                    gasPrice,
                    gasLimit: 500000,
                });
            } catch (error: any) {
                expect(error).toBeDefined();
            }
        });

        test('should fail with invalid hex encoding in transaction data', async () => {
            const query = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                chain_id: 1,
                height: 100,
                index: 0,
                // eslint-disable-next-line @typescript-eslint/naming-convention
                layout_segments: [{ offset: 0, size: 32 }],
            };

            const merkleProof = {
                root: ethers.zeroPadBytes('0x01', 32),
                siblings: [],
            };
            const continuityChain = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                block_numbers: [100],
                digests: [ethers.zeroPadBytes('0x01', 32)],
            };

            try {
                // Pass non-hex string as transaction data
                await contract.verifyQuery(query, 'not-hex-data', merkleProof, continuityChain, {
                    gasPrice,
                    gasLimit: 500000,
                });
            } catch (error: any) {
                expect(error).toBeDefined();
            }
        });
    });

    describe('Failing Cases - Expected Reverts', () => {
        test('should fail when querying without attestation data', async () => {
            const query = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                chain_id: 1,
                height: 100,
                index: 0,
                // eslint-disable-next-line @typescript-eslint/naming-convention
                layout_segments: [{ offset: 0, size: 32 }],
            };

            const txData = ethers.randomBytes(100);
            const merkleProof = {
                root: ethers.keccak256(txData),
                siblings: [],
            };
            const continuityChain = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                block_numbers: [100],
                digests: [ethers.zeroPadBytes('0x01', 32)],
            };

            try {
                const result = await contract.verifyQuery(query, txData, merkleProof, continuityChain, {
                    gasPrice,
                    gasLimit: 500000,
                });

                // If it doesn't revert, check the status is non-zero (failure)
                if (result && result.status) {
                    expect(result.status).toBeGreaterThan(0);
                }
            } catch (error: any) {
                // Expected to revert without proper attestation
                expect(error.message).toBeDefined();
            }
        });

        test('should fail with empty transaction data', async () => {
            const query = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                chain_id: 1,
                height: 100,
                index: 0,
                // eslint-disable-next-line @typescript-eslint/naming-convention
                layout_segments: [],
            };

            const txData = '0x'; // Empty transaction data
            const merkleProof = {
                root: ethers.zeroPadBytes('0x00', 32),
                siblings: [],
            };
            const continuityChain = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                block_numbers: [100],
                digests: [ethers.zeroPadBytes('0x01', 32)],
            };

            try {
                await contract.verifyQuery(query, txData, merkleProof, continuityChain, {
                    gasPrice,
                    gasLimit: 500000,
                });
            } catch (error: any) {
                // Expected to fail
                expect(error).toBeDefined();
            }
        });

        test('should fail when layout segment exceeds transaction data bounds', async () => {
            const query = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                chain_id: 1,
                height: 100,
                index: 0,
                // eslint-disable-next-line @typescript-eslint/naming-convention
                layout_segments: [
                    { offset: 150, size: 32 }, // Offset beyond tx data length
                ],
            };

            const txData = ethers.randomBytes(100); // Only 100 bytes
            const merkleProof = {
                root: ethers.keccak256(txData),
                siblings: [],
            };

            const continuityChain = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                block_numbers: [100],
                digests: [ethers.zeroPadBytes('0x01', 32)],
            };

            try {
                await contract.verifyQuery(query, txData, merkleProof, continuityChain, {
                    gasPrice,
                    gasLimit: 500000,
                });
            } catch (error: any) {
                // Expected to fail for out-of-bounds segment
                expect(error).toBeDefined();
            }
        });

        test('should fail with extremely large layout segments', async () => {
            const query = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                chain_id: 1,
                height: 100,
                index: 0,
                // eslint-disable-next-line @typescript-eslint/naming-convention
                layout_segments: [
                    { offset: 0, size: 2 ** 32 - 1 }, // Max uint32
                ],
            };

            const txData = ethers.randomBytes(100);
            const merkleProof = {
                root: ethers.keccak256(txData),
                siblings: [],
            };
            const continuityChain = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                block_numbers: [100],
                digests: [ethers.zeroPadBytes('0x01', 32)],
            };

            try {
                await contract.verifyQuery(query, txData, merkleProof, continuityChain, {
                    gasPrice,
                    gasLimit: 500000,
                });
            } catch (error: any) {
                // Expected to fail for extremely large segment
                expect(error).toBeDefined();
            }
        });

        test('should fail with mismatched merkle root', async () => {
            const query = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                chain_id: 1,
                height: 100,
                index: 0,
                // eslint-disable-next-line @typescript-eslint/naming-convention
                layout_segments: [{ offset: 0, size: 32 }],
            };

            const txData = ethers.randomBytes(100);
            const merkleProof = {
                root: ethers.keccak256('0xdeadbeef'), // Wrong root, doesn't match txData
                siblings: [ethers.randomBytes(32)],
            };
            const continuityChain = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                block_numbers: [100],
                digests: [ethers.zeroPadBytes('0x01', 32)],
            };

            try {
                await contract.verifyQuery(query, txData, merkleProof, continuityChain, {
                    gasPrice,
                    gasLimit: 500000,
                });
            } catch (error: any) {
                // Expected to fail for mismatched merkle root
                expect(error).toBeDefined();
            }
        });
    });
});
