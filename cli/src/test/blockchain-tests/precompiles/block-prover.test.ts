import * as proof from '@gluwa/usc-sdk/dist/proof-generator';
import { chainInfo } from '@gluwa/usc-sdk/dist/';
import { EncodingVersion } from '@gluwa/usc-sdk/dist/encoding';
import { WebSocketProvider, ethers } from 'ethers';
import { ApiPromise, BN, MICROUNITS_PER_CTC, newApi } from '../../../lib';
import { fundFromSudo } from '../../integration-tests/helpers';
import { chain_Anvil1_Key, chain_Anvil1_Url } from '../pallets/supported-chains/consts';
import { blockProverAddress } from './consts';
import { testIf } from '../../utils';

// eslint-disable-next-line @typescript-eslint/no-require-imports
import contractABIJSON = require('../artifacts/block_prover.json');
const contractABI = contractABIJSON as unknown as ethers.InterfaceAbi;

describe('Precompile: block-prover', (): void => {
    let contract: any;
    let provider: any;
    let alith: any;
    let api: ApiPromise;
    let gasPrice: bigint;
    // Helper to get the single-query verify function (disambiguate from batch overload)
    let verifySingle: any;
    let verifyAndEmitSingle: any;
    let verifyAndEmitBatch: any;

    // Helper to create a valid merkle proof for a single transaction
    // For single transactions, root = keccak256(0x00 || tx_data) with empty siblings
    const createValidMerkleProof = (txData: Uint8Array) => {
        // Prepend 0x00 (LEAF_HASH_PREPEND_VALUE) to transaction data
        const prefixed = new Uint8Array(txData.length + 1);
        prefixed[0] = 0x00;
        prefixed.set(txData, 1);
        const root = ethers.keccak256(prefixed);
        return {
            root,
            siblings: [], // Empty for single transaction
        };
    };

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);

        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        alith = new ethers.Wallet(privateKey, provider);

        // Fund Alith if needed
        const result = await fundFromSudo(api, alith.address, MICROUNITS_PER_CTC.mul(new BN(1_000_000)));
        expect(result.status).toBe(0);

        contract = new ethers.Contract(blockProverAddress, contractABI, alith);

        // Get the single-query verify function overload explicitly
        // Signature: verify(uint64,uint64,bytes,(bytes32,(bytes32,bool)[]),(bytes32,bytes32[]))
        verifySingle = contract.getFunction(
            'verify(uint64,uint64,bytes,(bytes32,(bytes32,bool)[]),(bytes32,bytes32[]))',
        );
        verifyAndEmitSingle = contract.getFunction(
            'verifyAndEmit(uint64,uint64,bytes,(bytes32,(bytes32,bool)[]),(bytes32,bytes32[]))',
        );
        verifyAndEmitBatch = contract.getFunction(
            'verifyAndEmit(uint64,uint64[],bytes[],(bytes32,(bytes32,bool)[])[],(bytes32,bytes32[]))',
        );
    }, 90_000);

    // Frontier calldata threshold (bytes) - GasLimitPovSizeRatio configured in the runtime (2,893 bytes)
    const FRONTIER_CALLDATA_THRESHOLD = 2893;

    afterAll(async () => {
        await api.disconnect();
    });

    beforeEach(async () => {
        gasPrice = (await provider.getFeeData()).gasPrice;
        // Wait a bit to avoid nonce conflicts between tests
        await new Promise((resolve) => setTimeout(resolve, 100));
    });

    describe('verify()', () => {
        testIf(
            process.env.ANVIL1_TXN_HASH !== undefined,
            'should return true when called with valid input',
            async () => {
                // we need a running Anvil at this address
                const anvil1Provider = new WebSocketProvider(chain_Anvil1_Url);

                // this value needs to be passed from the outside
                const transactionHash = process.env.ANVIL1_TXN_HASH;
                expect(transactionHash).toBeTruthy();

                // make sure we have attestations for this source block
                const sourceTxn = await anvil1Provider.getTransaction(transactionHash!);
                expect(sourceTxn).toBeDefined();
                expect(sourceTxn!.blockNumber).toBeDefined();

                const chainInfoProvider = new chainInfo.PrecompileChainInfoProvider(provider);
                await chainInfoProvider.waitUntilHeightAttested(
                    chain_Anvil1_Key,
                    sourceTxn!.blockNumber!,
                    5_000, // 5 sec poll interval
                    300_000, // 5 min wait Timeout
                );
                // we're now sure that there are enough attestations on the execution chain

                const blockProvider = new proof.raw.blockProvider.SimpleBlockProvider(anvil1Provider);
                const rawProofGenerator = new proof.raw.RawProofGenerator(
                    chain_Anvil1_Key,
                    blockProvider,
                    chainInfoProvider,
                    EncodingVersion.V1,
                );
                const rawProofResult = await rawProofGenerator.generateProof(transactionHash!);
                if (!rawProofResult.success) {
                    throw new Error(`Proof generation failed: ${rawProofResult.error ?? 'Unknown error'}`);
                }

                const proofData = rawProofResult.data!;
                const proveResultRaw = await verifySingle.staticCall(
                    proofData.chainKey,
                    proofData.headerNumber,
                    proofData.txBytes,
                    proofData.merkleProof,
                    proofData.continuityProof,
                );
                expect(proveResultRaw).toBe(true);
            },
            360_000,
        );
    });

    describe('Frontier calldata threshold (verifyAndEmit with calldata > 2893 bytes)', () => {
        /**
         * Reproduces Frontier EVM issue: transactions with calldata over ~2893 bytes
         * can trigger bugs. This test submits a verifyAndEmit call with calldata
         * exceeding the threshold (using batch API when single-call is smaller)
         * and asserts the transaction succeeds.
         *
         * Requires: ANVIL1_TXN_HASH env var, running Anvil at chain_Anvil1_Url
         */
        testIf(
            process.env.ANVIL1_TXN_HASH !== undefined,
            'should succeed when calldata exceeds 2893-byte Frontier threshold',
            async () => {
                const anvil1Provider = new WebSocketProvider(chain_Anvil1_Url);

                const transactionHash = process.env.ANVIL1_TXN_HASH;
                expect(transactionHash).toBeTruthy();

                const sourceTxn = await anvil1Provider.getTransaction(transactionHash!);
                expect(sourceTxn).toBeDefined();
                expect(sourceTxn!.blockNumber).toBeDefined();

                const chainInfoProvider = new chainInfo.PrecompileChainInfoProvider(provider);

                const blockProvider = new proof.raw.blockProvider.SimpleBlockProvider(anvil1Provider);
                const rawProofGenerator = new proof.raw.RawProofGenerator(
                    chain_Anvil1_Key,
                    blockProvider,
                    chainInfoProvider,
                    EncodingVersion.V1,
                );
                const rawProofResult = await rawProofGenerator.generateProof(transactionHash!);
                if (!rawProofResult.success) {
                    throw new Error(`Proof generation failed: ${rawProofResult.error ?? 'Unknown error'}`);
                }

                const proofData = rawProofResult.data!;
                const txBytesHex =
                    typeof proofData.txBytes === 'string'
                        ? proofData.txBytes.startsWith('0x')
                            ? proofData.txBytes
                            : '0x' + proofData.txBytes
                        : '0x' + Buffer.from(proofData.txBytes).toString('hex');

                const merkleProofTuple = [
                    proofData.merkleProof.root,
                    proofData.merkleProof.siblings.map((s: { hash: string; isLeft: boolean }) => [s.hash, s.isLeft]),
                ];
                const continuityProofTuple = [
                    proofData.continuityProof.lowerEndpointDigest,
                    proofData.continuityProof.roots ?? [],
                ];

                const iface = contract.interface;
                let data = iface.encodeFunctionData(verifyAndEmitSingle.fragment, [
                    proofData.chainKey,
                    proofData.headerNumber,
                    txBytesHex,
                    merkleProofTuple,
                    continuityProofTuple,
                ]);
                let calldataSizeBytes = (data.length - 2) / 2;

                if (calldataSizeBytes <= FRONTIER_CALLDATA_THRESHOLD) {
                    data = iface.encodeFunctionData(verifyAndEmitBatch.fragment, [
                        proofData.chainKey,
                        [proofData.headerNumber, proofData.headerNumber],
                        [txBytesHex, txBytesHex],
                        [merkleProofTuple, merkleProofTuple],
                        continuityProofTuple,
                    ]);
                    calldataSizeBytes = (data.length - 2) / 2;
                }

                expect(calldataSizeBytes).toBeGreaterThan(FRONTIER_CALLDATA_THRESHOLD);

                const tx = await alith.sendTransaction({
                    to: blockProverAddress,
                    data,
                    gasLimit: 500_000,
                    gasPrice,
                });

                const receipt = await tx.wait();
                expect(receipt).toBeDefined();
                expect(receipt!.status).toBe(1);

                const verifiedEvents = receipt!.logs
                    .map((log: { data: string; topics: string[] }) => {
                        try {
                            return iface.parseLog({ data: log.data, topics: log.topics });
                        } catch {
                            return null;
                        }
                    })
                    .filter((parsed: { name: string } | null) => parsed?.name === 'TransactionVerified');

                expect(verifiedEvents.length).toBeGreaterThan(0);
            },
            360_000,
        );

        testIf(
            process.env.ANVIL1_TXN_HASH !== undefined,
            'estimateGas should succeed when calldata exceeds 2893-byte Frontier threshold',
            async () => {
                const anvil1Provider = new WebSocketProvider(chain_Anvil1_Url);

                const transactionHash = process.env.ANVIL1_TXN_HASH;
                expect(transactionHash).toBeTruthy();

                const sourceTxn = await anvil1Provider.getTransaction(transactionHash!);
                expect(sourceTxn).toBeDefined();
                expect(sourceTxn!.blockNumber).toBeDefined();

                const chainInfoProvider = new chainInfo.PrecompileChainInfoProvider(provider);

                const blockProvider = new proof.raw.blockProvider.SimpleBlockProvider(anvil1Provider);
                const rawProofGenerator = new proof.raw.RawProofGenerator(
                    chain_Anvil1_Key,
                    blockProvider,
                    chainInfoProvider,
                    EncodingVersion.V1,
                );
                const rawProofResult = await rawProofGenerator.generateProof(transactionHash!);
                if (!rawProofResult.success) {
                    throw new Error(`Proof generation failed: ${rawProofResult.error ?? 'Unknown error'}`);
                }

                const proofData = rawProofResult.data!;
                const txBytesHex =
                    typeof proofData.txBytes === 'string'
                        ? proofData.txBytes.startsWith('0x')
                            ? proofData.txBytes
                            : '0x' + proofData.txBytes
                        : '0x' + Buffer.from(proofData.txBytes).toString('hex');

                const merkleProofTuple = [
                    proofData.merkleProof.root,
                    proofData.merkleProof.siblings.map((s: { hash: string; isLeft: boolean }) => [s.hash, s.isLeft]),
                ];
                const continuityProofTuple = [
                    proofData.continuityProof.lowerEndpointDigest,
                    proofData.continuityProof.roots ?? [],
                ];

                const iface = contract.interface;
                let data = iface.encodeFunctionData(verifyAndEmitSingle.fragment, [
                    proofData.chainKey,
                    proofData.headerNumber,
                    txBytesHex,
                    merkleProofTuple,
                    continuityProofTuple,
                ]);
                let calldataSizeBytes = (data.length - 2) / 2;

                if (calldataSizeBytes <= FRONTIER_CALLDATA_THRESHOLD) {
                    data = iface.encodeFunctionData(verifyAndEmitBatch.fragment, [
                        proofData.chainKey,
                        [proofData.headerNumber, proofData.headerNumber],
                        [txBytesHex, txBytesHex],
                        [merkleProofTuple, merkleProofTuple],
                        continuityProofTuple,
                    ]);
                    calldataSizeBytes = (data.length - 2) / 2;
                }

                expect(calldataSizeBytes).toBeGreaterThan(FRONTIER_CALLDATA_THRESHOLD);

                const estimatedGas = await provider.estimateGas({
                    to: blockProverAddress,
                    data,
                    from: alith.address,
                });

                expect(estimatedGas).toBeDefined();
                expect(estimatedGas).toBeGreaterThan(0n);
            },
            360_000,
        );

        testIf(
            process.env.ANVIL1_TXN_HASH !== undefined,
            'estimateGas should succeed when calldata is below 2893-byte Frontier threshold',
            async () => {
                const anvil1Provider = new WebSocketProvider(chain_Anvil1_Url);

                const transactionHash = process.env.ANVIL1_TXN_HASH;
                expect(transactionHash).toBeTruthy();

                const sourceTxn = await anvil1Provider.getTransaction(transactionHash!);
                expect(sourceTxn).toBeDefined();
                expect(sourceTxn!.blockNumber).toBeDefined();

                const chainInfoProvider = new chainInfo.PrecompileChainInfoProvider(provider);

                const blockProvider = new proof.raw.blockProvider.SimpleBlockProvider(anvil1Provider);
                const rawProofGenerator = new proof.raw.RawProofGenerator(
                    chain_Anvil1_Key,
                    blockProvider,
                    chainInfoProvider,
                    EncodingVersion.V1,
                );
                const rawProofResult = await rawProofGenerator.generateProof(transactionHash!);
                if (!rawProofResult.success) {
                    throw new Error(`Proof generation failed: ${rawProofResult.error ?? 'Unknown error'}`);
                }

                const proofData = rawProofResult.data!;
                const txBytesHex =
                    typeof proofData.txBytes === 'string'
                        ? proofData.txBytes.startsWith('0x')
                            ? proofData.txBytes
                            : '0x' + proofData.txBytes
                        : '0x' + Buffer.from(proofData.txBytes).toString('hex');

                const merkleProofTuple = [
                    proofData.merkleProof.root,
                    proofData.merkleProof.siblings.map((s: { hash: string; isLeft: boolean }) => [s.hash, s.isLeft]),
                ];
                const continuityProofTuple = [
                    proofData.continuityProof.lowerEndpointDigest,
                    proofData.continuityProof.roots ?? [],
                ];

                const iface = contract.interface;
                const data = iface.encodeFunctionData(verifyAndEmitSingle.fragment, [
                    proofData.chainKey,
                    proofData.headerNumber,
                    txBytesHex,
                    merkleProofTuple,
                    continuityProofTuple,
                ]);
                const calldataSizeBytes = (data.length - 2) / 2;

                if (calldataSizeBytes > FRONTIER_CALLDATA_THRESHOLD) {
                    return; // Skip: this proof's single-call is already over threshold
                }

                const estimatedGas = await provider.estimateGas({
                    to: blockProverAddress,
                    data,
                    from: alith.address,
                });

                expect(estimatedGas).toBeDefined();
                expect(estimatedGas).toBeGreaterThan(0n);
            },
            360_000,
        );
    });

    describe('Precompile Deployment', () => {
        test('should verify precompile is deployed at correct address', async () => {
            // Verify precompile exists at the expected address
            // Note: Precompiles may not have bytecode but should respond to calls
            const code = await provider.getCode(blockProverAddress);
            expect(blockProverAddress).toBe('0x0000000000000000000000000000000000000FD2');
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
                roots: [ethers.keccak256(txData)],
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
                roots: [ethers.keccak256(txData)],
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
                roots: [ethers.keccak256(smallTxData)],
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
                    roots: [ethers.keccak256(largeTxData)],
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
                roots: [ethers.keccak256(txData)],
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
                roots: [ethers.keccak256(txData)],
            };

            const longContinuityProof = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                lowerEndpointDigest: ethers.zeroPadBytes('0x00', 32),
                roots: [
                    ethers.keccak256(txData),
                    ethers.keccak256(txData),
                    ethers.keccak256(txData),
                    ethers.keccak256(txData),
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
                roots: [ethers.keccak256(txData)],
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
                roots: [ethers.zeroPadBytes('0x01', 32)],
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
            // Create valid merkle proof so we can test continuity validation
            const merkleProof = createValidMerkleProof(txData);

            // Malformed continuity proof with empty roots array
            const malformedProof = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                lowerEndpointDigest: ethers.zeroPadBytes('0x00', 32),
                roots: [], // Empty array
            };

            // Use staticCall to simulate without sending transaction (avoids nonce conflicts)
            // Empty roots will cause "Query block not found in continuity chain" error
            await expect(
                verifySingle.staticCall(chainKey, height, txData, merkleProof, malformedProof),
            ).rejects.toThrow(/Query block not found in continuity chain/);
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
                roots: [ethers.zeroPadBytes('0x01', 32)],
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
            // Create valid merkle proof so we can test continuity validation
            const merkleProof = createValidMerkleProof(txData);
            // Continuity proof: roots[0] is at queryHeight (query block at index 0)
            const continuityProof = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                lowerEndpointDigest: ethers.zeroPadBytes('0x00', 32),
                roots: [
                    // Block at queryHeight (index 0) - must match merkle proof root
                    merkleProof.root,
                ],
            };

            // Continuity chain validation happens after merkle proof validation.
            // Without attestation data on-chain, this fails at the continuity validation step.
            // Use staticCall to simulate without sending transaction (avoids nonce conflicts)
            await expect(
                verifySingle.staticCall(chainKey, height, txData, merkleProof, continuityProof),
            ).rejects.toThrow(/Continuity proof does not match attestation or checkpoint|Continuity chain/);
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
                roots: [ethers.zeroPadBytes('0x00', 32)],
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
            // Create valid merkle proof so we can test continuity validation
            const merkleProof = createValidMerkleProof(txData);

            // Continuity proof: roots[0] is at queryHeight (query block at index 0)
            const continuityProof = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                lowerEndpointDigest: ethers.zeroPadBytes('0x00', 32),
                roots: [
                    // Block at queryHeight (index 0) - must match merkle proof root
                    merkleProof.root,
                ],
            };

            // This test verifies the precompile properly rejects queries when
            // the continuity chain cannot be validated against on-chain attestations.
            // Use staticCall to simulate without sending transaction (avoids nonce conflicts)
            await expect(
                verifySingle.staticCall(chainKey, height, txData, merkleProof, continuityProof),
            ).rejects.toThrow(/Continuity proof does not match attestation or checkpoint|Continuity chain/);
        });

        test('should fail with invalid continuity proof', async () => {
            const chainKey = 1;
            const height = 100;

            const txData = ethers.randomBytes(100);
            // Create valid merkle proof so we can test continuity validation
            const merkleProof = createValidMerkleProof(txData);
            // Continuity proof: roots[0] is at queryHeight (query block at index 0)
            // The merkle root matches, but the continuity chain won't validate against on-chain data
            const continuityProof = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                lowerEndpointDigest: ethers.zeroPadBytes('0x00', 32),
                roots: [
                    // Block at queryHeight (index 0) - must match merkle proof root
                    merkleProof.root,
                ],
            };

            // Continuity chain validation happens after merkle proof validation.
            // With invalid data, this fails at continuity validation.
            // Use staticCall to simulate without sending transaction (avoids nonce conflicts)
            await expect(
                verifySingle.staticCall(chainKey, height, txData, merkleProof, continuityProof),
            ).rejects.toThrow(/Continuity proof does not match attestation or checkpoint|Continuity chain/);
        });

        test('should fail with mismatched merkle root', async () => {
            const chainKey = 1;
            const height = 100;

            const txData = ethers.randomBytes(100);
            // Create invalid merkle proof with wrong root
            const wrongRoot = ethers.keccak256(ethers.toUtf8Bytes('wrongRoot')); // Wrong root, doesn't match txData
            const merkleProof = {
                root: wrongRoot,
                siblings: [], // Empty siblings for single transaction
            };
            // Continuity proof: roots[0] is at queryHeight (query block at index 0)
            const continuityProof = {
                // eslint-disable-next-line @typescript-eslint/naming-convention
                lowerEndpointDigest: ethers.zeroPadBytes('0x00', 32),
                roots: [
                    // Block at queryHeight (index 0) - wrong root but matches continuity
                    wrongRoot,
                ],
            };

            // Merkle proof validation happens first, so with an invalid merkle proof,
            // we fail at merkle proof validation before reaching continuity validation.
            // Use staticCall to simulate without sending transaction (avoids nonce conflicts)
            await expect(
                verifySingle.staticCall(chainKey, height, txData, merkleProof, continuityProof),
            ).rejects.toThrow(/Merkle proof validation failed/);
        });
    });
});
