// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title INativeQueryVerifier
/// @notice Interface for the Native Query Verifier Precompile at address 0x0FD2 (4050)
/// @dev This precompile provides native-speed verification of blockchain queries using
///      Merkle proofs and continuity chains.
interface INativeQueryVerifier {
    /// @notice Query structure defining what data to retrieve from a blockchain
    /// @dev Specifies the chain, block, and data segments to extract from transaction data
    struct Query {
        /// Chain identifier (e.g., 1 for Ethereum mainnet)
        uint64 chain_id;
        /// Block height/number
        uint64 height;
        /// Data segments to extract from the transaction
        LayoutSegment[] layout_segments;
    }

    /// @notice Layout segment defining a specific data location within a transaction
    /// @dev Used to extract specific bytes from transaction data
    struct LayoutSegment {
        /// Byte offset in the transaction data
        uint64 offset;
        /// Number of bytes to extract
        uint64 size;
    }

    /// @notice Entry in a Merkle proof containing hash and position
    struct MerkleProofEntry {
        /// The sibling hash at this level
        bytes32 hash;
        /// Whether this sibling is on the left (true) or right (false)
        bool isLeft;
    }

    /// @notice Merkle proof for verifying transaction inclusion in a block
    /// @dev Contains the Merkle root and sibling entries with position information
    struct MerkleProof {
        /// The Merkle root hash of the transaction tree
        bytes32 root;
        /// Sibling entries for the Merkle proof path (with position info)
        MerkleProofEntry[] siblings;
    }

    /// @notice Optimized continuity block structure
    /// @dev Contains only root and digest (block_number and prev_digest are inferred)
    struct ContinuityBlock {
        /// Block root hash
        bytes32 root;
        /// Current block digest
        bytes32 digest;
    }

    /// @notice Optimized continuity proof structure
    /// @dev Reduces calldata size by removing redundant fields:
    ///      - block_number is inferred from query height(s) and index
    ///        Single query: blocks[0] is at queryHeight - 1
    ///        Batch queries: blocks[0] is at min(queryHeights) - 1
    struct ContinuityProof {
        /// The digest of the block before the continuity chain starts
        /// This is the prev_digest of the first block
        bytes32 lowerEndpointDigest;
        /// Array of continuity blocks (each containing only root and digest)
        /// Block numbers are inferred: blocks[i] is at (queryHeight - 1) + i for single query
        /// prev_digest is reconstructed from the chain (using lowerEndpointDigest and computed digests)
        ContinuityBlock[] blocks;
    }

    /// @notice Result of query verification
    /// @dev Contains the verification status and extracted data segments
    struct QueryVerificationResult {
        /// Verification status code:
        /// 0 = Success
        /// 1 = MerkleProofInvalid
        /// 2 = ContinuityChainInvalid
        /// 3 = DataExtractionError
        /// 4 = MerkleRootMismatch
        uint8 status;
        /// Extracted data segments from the verified transaction
        ResultSegment[] result_segments;
    }

    /// @notice A segment of extracted data from a verified transaction
    /// @dev Contains the offset and extracted bytes
    struct ResultSegment {
        /// Offset in the transaction data where this segment was extracted
        uint64 offset;
        /// Extracted bytes (32-byte chunks)
        /// NOTE: Named 'bytes' in the actual ABI for compatibility with precompile
        /// but 'bytes' is a Solidity reserved keyword, so we use 'data' here for compilation
        bytes32 data;
    }

    /// @notice Result of batch query verification
    /// @dev Contains statistics and individual results for each query
    struct BatchQueryVerificationResult {
        /// Number of successfully verified queries
        uint32 successful_queries;
        /// Number of failed queries
        uint32 failed_queries;
        /// Individual results for each query in the batch
        QueryVerificationResult[] results;
    }

    /// @notice Verify a blockchain query with Merkle proof and continuity chain (view function)
    /// @dev This is a read-only view function that performs native verification at runtime speed.
    ///      This function does NOT emit events. Use verifyQuery() if you need event emissions.
    /// @param query The query specification defining what data to retrieve
    /// @param tx_data Raw transaction data to verify and extract from
    /// @param merkle_proof Merkle proof for transaction inclusion (with position info, no index needed)
    /// @param continuity_proof Optimized continuity proof (blocks[0] is at queryHeight-1)
    /// @return result Verification result containing status and extracted data segments
    ///
    /// Gas Costs (aligned with standard Ethereum precompiles):
    /// - Base: 21,000 (matches Ethereum standard)
    /// - Per TX byte: 16 (matches EVM calldata cost)
    /// - Per sibling: 200 (native efficiency)
    /// - Per continuity block: 3,000
    /// - Storage lookup: 2,600 per attestation/checkpoint (matches cold SLOAD)
    /// - Merkle verification: 100,000 weight
    /// - Continuity verification: 50,000 weight
    ///
    /// Note: This function does not emit events. For event emissions, use verifyQuery() instead.
    function verifyQueryView(
        Query calldata query,
        bytes calldata tx_data,
        MerkleProof calldata merkle_proof,
        ContinuityProof calldata continuity_proof
    ) external view returns (QueryVerificationResult memory result);

    /// @notice Verify a blockchain query with Merkle proof and continuity chain
    /// @dev This is the state-changing version that emits QueryVerified or QueryVerificationFailed events.
    ///      For read-only verification without events, use verifyQueryView() instead.
    /// @param query The query specification defining what data to retrieve
    /// @param tx_data Raw transaction data to verify and extract from
    /// @param merkle_proof Merkle proof for transaction inclusion (with position info, no index needed)
    /// @param continuity_proof Optimized continuity proof (blocks[0] is at queryHeight-1)
    /// @return result Verification result containing status and extracted data segments
    ///
    /// Events Emitted:
    /// - QueryVerified on successful verification
    /// - QueryVerificationFailed on verification failure (before revert)
    ///
    /// Gas Costs (aligned with standard Ethereum precompiles):
    /// - Base: 21,000 (matches Ethereum standard)
    /// - Per TX byte: 16 (matches EVM calldata cost)
    /// - Per sibling: 200 (native efficiency)
    /// - Per continuity block: 3,000
    /// - Storage lookup: 2,600 per attestation/checkpoint (matches cold SLOAD)
    /// - Merkle verification: 100,000 weight
    /// - Continuity verification: 50,000 weight
    /// - Event emission: Additional gas for log costs
    ///
    /// Example Usage:
    /// ```solidity
    /// INativeQueryVerifier verifier = INativeQueryVerifier(0x0000000000000000000000000000000000000FD2);
    ///
    /// // Define data segments to extract (e.g., ERC20 transfer event)
    /// INativeQueryVerifier.LayoutSegment[] memory segments = new INativeQueryVerifier.LayoutSegment[](2);
    /// segments[0] = INativeQueryVerifier.LayoutSegment(192, 32);  // from address
    /// segments[1] = INativeQueryVerifier.LayoutSegment(224, 32);  // to address
    ///
    /// // Create query (no transaction index needed!)
    /// INativeQueryVerifier.Query memory query = INativeQueryVerifier.Query({
    ///     chain_id: 1,
    ///     height: 18000000,
    ///     layout_segments: segments
    /// });
    ///
    /// // Create merkle proof with position information
    /// INativeQueryVerifier.MerkleProofEntry[] memory siblings = new INativeQueryVerifier.MerkleProofEntry[](2);
    /// siblings[0] = INativeQueryVerifier.MerkleProofEntry(siblingHash1, false); // sibling on right
    /// siblings[1] = INativeQueryVerifier.MerkleProofEntry(siblingHash2, true);  // sibling on left
    ///
    /// INativeQueryVerifier.MerkleProof memory proof = INativeQueryVerifier.MerkleProof({
    ///     root: merkleRoot,
    ///     siblings: siblings
    /// });
    ///
    /// // Verify (emits QueryVerified event on success)
    /// INativeQueryVerifier.QueryVerificationResult memory result = verifier.verifyQuery(
    ///     query,
    ///     txData,
    ///     proof,
    ///     continuityProof
    /// );
    ///
    /// require(result.status == 0, "Verification failed");
    /// // Use result.result_segments...
    /// ```
    function verifyQuery(
        Query calldata query,
        bytes calldata tx_data,
        MerkleProof calldata merkle_proof,
        ContinuityProof calldata continuity_proof
    ) external returns (QueryVerificationResult memory result);

    /// @notice Verify a batch of queries with shared continuity proof (view function)
    /// @dev This is a read-only view function that optimizes gas costs by verifying the continuity
    ///      chain only once for all queries. Maximum batch size is 10 queries.
    ///      This function does NOT emit events. Use verifyBatchQueries() if you need event emissions.
    /// @param queries Array of queries to verify (max 10)
    /// @param tx_data_array Transaction data for each query
    /// @param merkle_proofs Merkle proofs for each query
    /// @param shared_continuity_proof Shared continuity proof covering all query heights
    /// @return result Batch verification result with statistics and individual results
    ///
    /// Gas Optimization:
    /// - Continuity chain is verified once for all queries instead of per-query
    /// - For 5 queries with 20-block continuity: saves ~240,000 gas (80% reduction)
    ///
    /// Note: This function does not emit events. For event emissions, use verifyBatchQueries() instead.
    ///
    /// Requirements:
    /// - All input arrays must have the same length
    /// - Batch size must not exceed 10 queries
    /// - Continuity chain must cover min to max query heights
    /// - Each query's merkle root must match its block in the continuity chain
    function verifyBatchQueriesView(
        Query[] calldata queries,
        bytes[] calldata tx_data_array,
        MerkleProof[] calldata merkle_proofs,
        ContinuityProof calldata shared_continuity_proof
    ) external view returns (BatchQueryVerificationResult memory result);

    /// @notice Verify a batch of queries with shared continuity proof
    /// @dev This is the state-changing version that emits events. This function optimizes gas costs
    ///      by verifying the continuity chain only once for all queries in the batch.
    ///      Maximum batch size is 10 queries.
    ///      IMPORTANT: Individual QueryVerified/QueryVerificationFailed events are emitted
    ///      for each query in addition to the BatchQueriesVerified summary event.
    /// @param queries Array of queries to verify (max 10)
    /// @param tx_data_array Transaction data for each query
    /// @param merkle_proofs Merkle proofs for each query
    /// @param shared_continuity_proof Shared continuity proof covering all query heights
    /// @return result Batch verification result with statistics and individual results
    ///
    /// Gas Optimization:
    /// - Continuity chain is verified once for all queries instead of per-query
    /// - For 5 queries with 20-block continuity: saves ~240,000 gas (80% reduction)
    ///
    /// Events Emitted:
    /// - QueryVerified or QueryVerificationFailed for each individual query
    /// - BatchQueriesVerified with summary statistics at the end
    ///
    /// Requirements:
    /// - All input arrays must have the same length
    /// - Batch size must not exceed 10 queries
    /// - Continuity chain must cover min to max query heights
    /// - Each query's merkle root must match its block in the continuity chain
    ///
    /// Example Usage:
    /// ```solidity
    /// INativeQueryVerifier verifier = INativeQueryVerifier(0x0000000000000000000000000000000000000FD2);
    ///
    /// // Create multiple queries
    /// INativeQueryVerifier.Query[] memory queries = new INativeQueryVerifier.Query[](3);
    /// queries[0] = createQuery(1, 100, segments1);
    /// queries[1] = createQuery(1, 101, segments2);
    /// queries[2] = createQuery(1, 102, segments3);
    ///
    /// // Prepare transaction data and proofs
    /// bytes[] memory txDataArray = new bytes[](3);
    /// INativeQueryVerifier.MerkleProof[] memory proofs = new INativeQueryVerifier.MerkleProof[](3);
    /// // ... fill arrays ...
    ///
    /// // Use shared continuity proof covering blocks 100-102
    /// INativeQueryVerifier.ContinuityProof memory sharedProof = getContinuityProof(100, 102);
    ///
    /// // Batch verify (emits events for each query + summary)
    /// INativeQueryVerifier.BatchQueryVerificationResult memory result = verifier.verifyBatchQueries(
    ///     queries,
    ///     txDataArray,
    ///     proofs,
    ///     sharedBlocks
    /// );
    ///
    /// require(result.failed_queries == 0, "Some queries failed");
    /// ```
    function verifyBatchQueries(
        Query[] calldata queries,
        bytes[] calldata tx_data_array,
        MerkleProof[] calldata merkle_proofs,
        ContinuityProof calldata shared_continuity_proof
    ) external returns (BatchQueryVerificationResult memory result);

    /// @notice Emitted when a query is successfully verified
    /// @param caller The address that initiated the verification
    /// @param queryId The unique identifier of the query
    /// @param chainKey The chain key from the query
    /// @param height The block height from the query
    /// @param status The verification status (0 for success)
    /// @param resultSegments The extracted data segments
    event QueryVerified(
        address indexed caller,
        bytes32 queryId,
        uint64 chainKey,
        uint64 height,
        uint8 status,
        ResultSegment[] resultSegments
    );

    /// @notice Emitted when query verification fails
    /// @param caller The address that initiated the verification
    /// @param queryId The unique identifier of the query
    /// @param chainKey The chain key from the query
    /// @param height The block height from the query
    /// @param status The verification status (non-zero for failure)
    /// @param reason The reason for verification failure
    event QueryVerificationFailed(
        address indexed caller,
        bytes32 queryId,
        uint64 chainKey,
        uint64 height,
        uint8 status,
        string reason
    );

    /// @notice Emitted when a batch of queries is verified
    /// @dev This is emitted in addition to individual QueryVerified/QueryVerificationFailed
    ///      events for each query in the batch
    /// @param successful Number of queries that succeeded
    /// @param failed Number of queries that failed
    /// @param total Total number of queries in the batch
    event BatchQueriesVerified(
        uint256 successful,
        uint256 failed,
        uint256 total
    );
}

/// @title NativeQueryVerifierLib
/// @notice Helper library for working with the Native Query Verifier precompile
/// @dev Provides convenience functions and constants
library NativeQueryVerifierLib {
    /// @notice Address of the Native Query Verifier precompile
    address constant PRECOMPILE_ADDRESS = 0x0000000000000000000000000000000000000FD2;

    /// @notice Status code: Verification successful
    uint8 constant STATUS_SUCCESS = 0;
    /// @notice Status code: Merkle proof verification failed
    uint8 constant STATUS_MERKLE_INVALID = 1;
    /// @notice Status code: Continuity chain validation failed
    uint8 constant STATUS_CONTINUITY_INVALID = 2;
    /// @notice Status code: Data extraction error
    uint8 constant STATUS_DATA_ERROR = 3;
    /// @notice Status code: Merkle root doesn't match continuity block
    uint8 constant STATUS_MERKLE_ROOT_MISMATCH = 4;

    /// @notice Get the precompile instance
    /// @return The INativeQueryVerifier interface instance
    function getVerifier() internal pure returns (INativeQueryVerifier) {
        return INativeQueryVerifier(PRECOMPILE_ADDRESS);
    }

    /// @notice Check if a verification result is successful
    /// @param result The verification result to check
    /// @return True if verification was successful
    function isSuccess(
        INativeQueryVerifier.QueryVerificationResult memory result
    ) internal pure returns (bool) {
        return result.status == STATUS_SUCCESS;
    }

    /// @notice Get a human-readable error message for a status code
    /// @param status The status code
    /// @return Error message string
    function getErrorMessage(
        uint8 status
    ) internal pure returns (string memory) {
        if (status == STATUS_SUCCESS) return "Success";
        if (status == STATUS_MERKLE_INVALID) return "Merkle proof invalid";
        if (status == STATUS_CONTINUITY_INVALID)
            return "Continuity chain invalid";
        if (status == STATUS_DATA_ERROR) return "Data extraction error";
        if (status == STATUS_MERKLE_ROOT_MISMATCH)
            return "Merkle root mismatch";
        return "Unknown error";
    }
}
