// SPDX-License-Identifier: MIT
pragma solidity ^0.8.19;

/**
 * @title GasBenchmark
 * @notice Contract for benchmarking gas costs of native query verification operations
 * @dev This contract simulates the operations performed by the native-query-verifier precompile
 *      to get accurate gas measurements for each step of the verification process
 */
contract GasBenchmark {
    // Events to log gas consumption at each step
    event GasConsumed(string operation, uint256 gasUsed);
    event VerificationComplete(uint256 totalGasUsed);

    // Struct to match the Query structure
    struct Query {
        uint64 chainId;
        uint64 height;
        uint64 index;
        LayoutSegment[] layoutSegments;
    }

    struct LayoutSegment {
        uint64 offset;
        uint64 size;
    }

    struct MerkleProof {
        bytes32 root;
        bytes32[] siblings;
    }

    struct ContinuityBlock {
        uint64 blockNumber;
        bytes32 root;
        bytes32 prevDigest;
        bytes32 digest;
    }

    struct ResultSegment {
        uint64 offset;
        bytes32 bytes_data;
    }

    /**
     * @notice Benchmark the complete verification flow
     * @param query The query parameters
     * @param txData The transaction data to verify
     * @param merkleProof The Merkle proof for transaction inclusion
     * @param continuityBlocks The continuity chain blocks
     * @return gasReport Detailed gas consumption for each operation
     */
    function benchmarkVerification(
        Query calldata query,
        bytes calldata txData,
        MerkleProof calldata merkleProof,
        ContinuityBlock[] calldata continuityBlocks
    ) external returns (string memory gasReport) {
        uint256 startGas = gasleft();
        uint256 gasCheckpoint;

        // Step 1: Benchmark Merkle proof verification
        gasCheckpoint = gasleft();
        bool merkleValid = benchmarkMerkleVerification(
            txData,
            merkleProof,
            query.index
        );
        uint256 merkleGas = gasCheckpoint - gasleft();
        emit GasConsumed("MerkleVerification", merkleGas);

        // Step 2: Benchmark continuity chain verification
        gasCheckpoint = gasleft();
        bool continuityValid = benchmarkContinuityVerification(
            continuityBlocks,
            query
        );
        uint256 continuityGas = gasCheckpoint - gasleft();
        emit GasConsumed("ContinuityVerification", continuityGas);

        // Step 3: Benchmark data extraction
        gasCheckpoint = gasleft();
        ResultSegment[] memory segments = benchmarkDataExtraction(
            txData,
            query.layoutSegments
        );
        uint256 extractionGas = gasCheckpoint - gasleft();
        emit GasConsumed("DataExtraction", extractionGas);

        uint256 totalGas = startGas - gasleft();
        emit VerificationComplete(totalGas);

        // Build detailed gas report
        gasReport = string(
            abi.encodePacked(
                "Gas Consumption Report:\n",
                "- Merkle Verification: ",
                uint2str(merkleGas),
                " gas\n",
                "  * Per sibling: ~",
                uint2str(merkleGas / (merkleProof.siblings.length + 1)),
                " gas\n",
                "- Continuity Verification: ",
                uint2str(continuityGas),
                " gas\n",
                "  * Per block: ~",
                uint2str(continuityGas / continuityBlocks.length),
                " gas\n",
                "- Data Extraction: ",
                uint2str(extractionGas),
                " gas\n",
                "  * Per segment: ~",
                uint2str(extractionGas / query.layoutSegments.length),
                " gas\n",
                "- Total: ",
                uint2str(totalGas),
                " gas"
            )
        );

        return gasReport;
    }

    /**
     * @notice Benchmark Merkle proof verification (simulated)
     * @dev In reality, this would use Pedersen hash, but we simulate with keccak256
     */
    function benchmarkMerkleVerification(
        bytes calldata txData,
        MerkleProof calldata proof,
        uint64 txIndex
    ) internal pure returns (bool) {
        // Simulate leaf hash computation (would be Pedersen in precompile)
        bytes32 currentHash = keccak256(abi.encodePacked(uint8(0x00), txData));

        // Handle single transaction case
        if (proof.siblings.length == 0) {
            return currentHash == proof.root;
        }

        // Traverse tree with siblings
        uint64 index = txIndex;
        uint256 arity = 2;
        uint256 numLevels = proof.siblings.length / arity;

        for (uint256 level = 0; level < numLevels; level++) {
            uint256 offset = index % arity;
            uint256 start = level * arity;

            // Simulate inner node hash (would be Pedersen in precompile)
            if (offset == 0) {
                currentHash = keccak256(
                    abi.encodePacked(
                        uint8(0x01),
                        currentHash,
                        proof.siblings[start + 1]
                    )
                );
            } else {
                currentHash = keccak256(
                    abi.encodePacked(
                        uint8(0x01),
                        proof.siblings[start],
                        currentHash
                    )
                );
            }

            index /= uint64(arity);
        }

        return currentHash == proof.root;
    }

    /**
     * @notice Benchmark continuity chain verification
     */
    function benchmarkContinuityVerification(
        ContinuityBlock[] calldata blocks,
        Query calldata query
    ) internal pure returns (bool) {
        if (blocks.length == 0) {
            return false;
        }

        // Simulate digest verification for each block
        for (uint256 i = 0; i < blocks.length; i++) {
            // Simulate digest computation (would be Pedersen in precompile)
            bytes32 expectedDigest = keccak256(
                abi.encodePacked(
                    blocks[i].blockNumber,
                    blocks[i].root,
                    blocks[i].prevDigest
                )
            );

            // In real verification, we'd check against stored attestations
            // Here we just verify the digest computation
            if (blocks[i].digest != expectedDigest) {
                // For benchmarking, we don't actually fail
                // return false;
            }

            // Verify chain continuity
            if (i > 0 && blocks[i].prevDigest != blocks[i - 1].digest) {
                // return false;
            }
        }

        return true;
    }

    /**
     * @notice Benchmark data extraction from transaction
     */
    function benchmarkDataExtraction(
        bytes calldata txData,
        LayoutSegment[] calldata segments
    ) internal pure returns (ResultSegment[] memory) {
        ResultSegment[] memory results = new ResultSegment[](segments.length);

        for (uint256 i = 0; i < segments.length; i++) {
            uint64 offset = segments[i].offset;
            uint64 size = segments[i].size;

            // Extract bytes at specified offset
            bytes memory extracted = new bytes(size);
            for (uint64 j = 0; j < size && offset + j < txData.length; j++) {
                extracted[j] = txData[offset + j];
            }

            // Convert to bytes32 (padding with zeros if needed)
            bytes32 result;
            assembly {
                result := mload(add(extracted, 32))
            }

            results[i] = ResultSegment({offset: offset, bytes_data: result});
        }

        return results;
    }

    /**
     * @notice Benchmark individual operations for granular measurements
     */

    function benchmarkPedersenHash(
        bytes calldata data
    ) external returns (uint256) {
        uint256 gasStart = gasleft();
        // Simulate Pedersen hash with keccak256
        bytes32 hash = keccak256(data);
        uint256 gasUsed = gasStart - gasleft();
        emit GasConsumed("PedersenHash", gasUsed);
        return gasUsed;
    }

    function benchmarkStorageLookup(bytes32 key) external returns (uint256) {
        uint256 gasStart = gasleft();
        // Simulate storage lookup
        bytes32 value = keccak256(abi.encodePacked(key, block.timestamp));
        uint256 gasUsed = gasStart - gasleft();
        emit GasConsumed("StorageLookup", gasUsed);
        return gasUsed;
    }

    function benchmarkMemcpy(
        bytes calldata data,
        uint64 offset,
        uint64 size
    ) external returns (uint256) {
        uint256 gasStart = gasleft();
        bytes memory extracted = new bytes(size);
        for (uint64 i = 0; i < size && offset + i < data.length; i++) {
            extracted[i] = data[offset + i];
        }
        uint256 gasUsed = gasStart - gasleft();
        emit GasConsumed("Memcpy", gasUsed);
        return gasUsed;
    }

    /**
     * @notice Run comprehensive benchmark suite
     */
    function runBenchmarkSuite() external returns (string memory report) {
        // Test different scenarios
        uint256[] memory costs = new uint256[](6);

        // Small transaction (100 bytes)
        costs[0] = measureScenario(100, 0, 1, 1);

        // Medium transaction (1KB)
        costs[1] = measureScenario(1024, 3, 5, 4);

        // Large transaction (10KB)
        costs[2] = measureScenario(10240, 7, 10, 8);

        // Deep Merkle tree (15 levels)
        costs[3] = measureScenario(1024, 15, 5, 4);

        // Long continuity chain (20 blocks)
        costs[4] = measureScenario(1024, 3, 20, 4);

        // Many data segments (16 segments)
        costs[5] = measureScenario(1024, 3, 5, 16);

        report = string(
            abi.encodePacked(
                "Benchmark Results:\n",
                "Small tx (100B): ",
                uint2str(costs[0]),
                " gas\n",
                "Medium tx (1KB): ",
                uint2str(costs[1]),
                " gas\n",
                "Large tx (10KB): ",
                uint2str(costs[2]),
                " gas\n",
                "Deep tree (15 levels): ",
                uint2str(costs[3]),
                " gas\n",
                "Long chain (20 blocks): ",
                uint2str(costs[4]),
                " gas\n",
                "Many segments (16): ",
                uint2str(costs[5]),
                " gas"
            )
        );

        return report;
    }

    function measureScenario(
        uint256 txSize,
        uint256 merkleDepth,
        uint256 continuityLength,
        uint256 segmentCount
    ) internal returns (uint256) {
        // Generate test data
        bytes memory txData = new bytes(txSize);
        for (uint256 i = 0; i < txSize; i++) {
            txData[i] = bytes1(uint8(i % 256));
        }

        // Build test structures
        MerkleProof memory proof = MerkleProof({
            root: keccak256(txData),
            siblings: new bytes32[](merkleDepth * 2)
        });

        ContinuityBlock[] memory blocks = new ContinuityBlock[](
            continuityLength
        );
        for (uint256 i = 0; i < continuityLength; i++) {
            blocks[i] = ContinuityBlock({
                blockNumber: uint64(i + 1),
                root: keccak256(abi.encodePacked(i)),
                prevDigest: i == 0
                    ? bytes32(0)
                    : keccak256(abi.encodePacked(i - 1)),
                digest: keccak256(abi.encodePacked(i))
            });
        }

        LayoutSegment[] memory segments = new LayoutSegment[](segmentCount);
        for (uint256 i = 0; i < segmentCount; i++) {
            segments[i] = LayoutSegment({offset: uint64(i * 32), size: 32});
        }

        Query memory query = Query({
            chainId: 2,
            height: 736,
            index: 0,
            layoutSegments: segments
        });

        // Measure gas
        uint256 gasStart = gasleft();
        benchmarkMerkleVerification(txData, proof, 0);
        benchmarkContinuityVerification(blocks, query);
        benchmarkDataExtraction(txData, segments);
        return gasStart - gasleft();
    }

    // Helper function to convert uint to string
    function uint2str(uint256 _i) internal pure returns (string memory) {
        if (_i == 0) {
            return "0";
        }
        uint256 j = _i;
        uint256 length;
        while (j != 0) {
            length++;
            j /= 10;
        }
        bytes memory bstr = new bytes(length);
        uint256 k = length;
        while (_i != 0) {
            k = k - 1;
            uint8 temp = (48 + uint8(_i - (_i / 10) * 10));
            bytes1 b1 = bytes1(temp);
            bstr[k] = b1;
            _i /= 10;
        }
        return string(bstr);
    }
}
