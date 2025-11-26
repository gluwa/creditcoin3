// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title INativeQueryVerifier
/// @notice Interface for the Native Query Verifier Precompile at address 0x0FD2 (4050)
/// @dev This precompile provides native-speed verification of blockchain queries using
///      Merkle proofs and continuity chains.
interface INativeQueryVerifier {
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
        bytes32 merkleRoot;
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

    /// @notice Emitted when a transaction is successfully verified
    /// @param chainKey The chain key identifier (indexed for efficient filtering)
    /// @param height The block height that was verified (indexed for efficient filtering)
    /// @param transactionIndex The transaction index calculated from Merkle proof siblings
    event TransactionVerified(
        uint64 indexed chainKey,
        uint64 indexed height,
        uint64 transactionIndex
    );

    /// @notice Verify a blockchain query with Merkle proof and continuity chain
    /// @dev This is the state-changing version that emits a TransactionVerified event on success.
    ///      Reverts on failure, returns true on success.
    /// @param chainKey The chain key identifier
    /// @param height The block height to verify
    /// @param encodedTransaction Raw transaction data to verify
    /// @param merkleProof Merkle proof for transaction inclusion (with position info)
    /// @param continuityProof Optimized continuity proof (blocks[0] is at queryHeight-1)
    /// @return true on successful verification (reverts on failure)
    ///
    /// Events Emitted:
    /// - TransactionVerified(uint64 chainKey, uint64 height, uint64 transactionIndex) on success
    ///
    /// Gas Costs (aligned with standard Ethereum precompiles):
    /// - Base: 21,000 (matches Ethereum standard)
    /// - Per TX byte: 16 (matches EVM calldata cost)
    /// - Per sibling: 200 (native efficiency)
    /// - Per continuity block: 400
    /// - Storage lookup: 2,600 per attestation/checkpoint (matches cold SLOAD)
    /// - Merkle verification: 100,000 weight
    /// - Continuity verification: 50,000 weight
    /// - Event emission: Additional gas for log costs
    ///
    /// Example Usage:
    /// ```solidity
    /// INativeQueryVerifier verifier = INativeQueryVerifier(0x0000000000000000000000000000000000000FD2);
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
    /// // Verify (emits TransactionVerified event on success, reverts on failure)
    /// bool success = verifier.verifyAndEmit(
    ///     1,      // chainKey
    ///     18000000, // height
    ///     encodedTransaction,
    ///     proof,
    ///     continuityProof
    /// );
    /// ```
    function verifyAndEmit(
        uint64 chainKey,
        uint64 height,
        bytes calldata encodedTransaction,
        MerkleProof calldata merkleProof,
        ContinuityProof calldata continuityProof
    ) external returns (bool);

    /// @notice Verify a batch of queries with shared continuity proof
    /// @dev This is the state-changing version that emits a TransactionVerified event for each successful transaction.
    ///      Optimized for batch verification by validating the continuity chain once.
    ///      Reverts on any failure, returns true if all verifications succeed.
    /// @param chainKey The chain key identifier (same for all queries)
    /// @param heights Array of block heights to verify
    /// @param encodedTransactions Transaction data for each query (must match heights length)
    /// @param merkleProofs Merkle proofs for each query (must match heights length)
    /// @param sharedContinuityProof Shared continuity proof covering all query heights
    /// @return true if all verifications succeed (reverts on any failure)
    ///
    /// Events Emitted:
    /// - TransactionVerified(uint64 chainKey, uint64 height, uint64 transactionIndex) for each successfully verified transaction
    ///
    /// Gas Optimization:
    /// - Continuity chain is verified once for all queries instead of per-query
    /// - For 5 queries with 20-block continuity: saves ~240,000 gas (80% reduction)
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
    /// uint64[] memory heights = new uint64[](3);
    /// heights[0] = 100;
    /// heights[1] = 101;
    /// heights[2] = 102;
    ///
    /// bytes[] memory encodedTransactions = new bytes[](3);
    /// INativeQueryVerifier.MerkleProof[] memory proofs = new INativeQueryVerifier.MerkleProof[](3);
    /// // ... fill arrays ...
    ///
    /// // Use shared continuity proof covering blocks 100-102
    /// bool success = verifier.verifyAndEmit(
    ///     1,              // chainKey
    ///     heights,        // heights array triggers batch overload
    ///     encodedTransactions,
    ///     proofs,
    ///     sharedContinuityProof
    /// );
    /// ```
    function verifyAndEmit(
        uint64 chainKey,
        uint64[] calldata heights,
        bytes[] calldata encodedTransactions,
        MerkleProof[] calldata merkleProofs,
        ContinuityProof calldata sharedContinuityProof
    ) external returns (bool);

    /// @notice Verify a single blockchain query with Merkle proof and continuity chain (view function)
    /// @dev This is a read-only view function that doesn't emit events. It charges the same gas
    ///      as the non-view function but doesn't modify state or emit logs.
    ///      Useful for off-chain verification or when events are not needed.
    /// @param chainKey The chain key identifier
    /// @param height The block height to verify
    /// @param encodedTransaction Raw transaction data to verify
    /// @param merkleProof Merkle proof for transaction inclusion (with position info)
    /// @param continuityProof Optimized continuity proof (blocks[0] is at queryHeight-1)
    /// @return true on successful verification (reverts on failure)
    ///
    /// Note: This function does not emit events. For event emissions, use verifyAndEmit() instead.
    ///
    /// Gas Costs (aligned with standard Ethereum precompiles):
    /// - Base: 21,000 (matches Ethereum standard)
    /// - Per TX byte: 16 (matches EVM calldata cost)
    /// - Per sibling: 200 (native efficiency)
    /// - Per continuity block: 400
    /// - Storage lookup: 2,600 per attestation/checkpoint (matches cold SLOAD)
    /// - Merkle verification: 100,000 weight
    /// - Continuity verification: 50,000 weight
    ///
    /// Example Usage:
    /// ```solidity
    /// INativeQueryVerifier verifier = INativeQueryVerifier(0x0000000000000000000000000000000000000FD2);
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
    /// // Verify (read-only, no events, reverts on failure)
    /// bool success = verifier.verify(
    ///     1,      // chainKey
    ///     18000000, // height
    ///     encodedTransaction,
    ///     proof,
    ///     continuityProof
    /// );
    /// ```
    function verify(
        uint64 chainKey,
        uint64 height,
        bytes calldata encodedTransaction,
        MerkleProof calldata merkleProof,
        ContinuityProof calldata continuityProof
    ) external view returns (bool);

    /// @notice Verify a batch of queries with shared continuity proof (view function)
    /// @dev This is a read-only view function that doesn't emit events. Optimized for batch
    ///      verification by validating the continuity chain once and reusing it for all queries.
    ///      This can save ~40% gas compared to individual verifications.
    /// @param chainKey The chain key identifier (same for all queries)
    /// @param heights Array of block heights to verify
    /// @param encodedTransactions Transaction data for each query (must match heights length)
    /// @param merkleProofs Merkle proofs for each query (must match heights length)
    /// @param sharedContinuityProof Shared continuity proof covering all query heights
    /// @return true if all verifications succeed (reverts on any failure)
    ///
    /// Note: This function does not emit events. For event emissions, use verifyAndEmit() instead.
    ///
    /// Gas Optimization:
    /// - Continuity chain is verified once for all queries instead of per-query
    /// - For 5 queries with 20-block continuity: saves ~240,000 gas (80% reduction)
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
    /// uint64[] memory heights = new uint64[](3);
    /// heights[0] = 100;
    /// heights[1] = 101;
    /// heights[2] = 102;
    ///
    /// bytes[] memory encodedTransactions = new bytes[](3);
    /// INativeQueryVerifier.MerkleProof[] memory proofs = new INativeQueryVerifier.MerkleProof[](3);
    /// // ... fill arrays ...
    ///
    /// // Use shared continuity proof covering blocks 100-102
    /// bool success = verifier.verify(
    ///     1,              // chainKey
    ///     heights,        // heights array triggers batch overload
    ///     encodedTransactions,
    ///     proofs,
    ///     sharedContinuityProof
    /// );
    /// ```
    function verify(
        uint64 chainKey,
        uint64[] calldata heights,
        bytes[] calldata encodedTransactions,
        MerkleProof[] calldata merkleProofs,
        ContinuityProof calldata sharedContinuityProof
    ) external view returns (bool);
}

/// @title NativeQueryVerifierLib
/// @notice Helper library for working with the Native Query Verifier precompile
/// @dev Provides convenience functions and constants
library NativeQueryVerifierLib {
    /// @notice Address of the Native Query Verifier precompile
    address constant PRECOMPILE_ADDRESS = 0x0000000000000000000000000000000000000FD2;

    /// @notice Get the precompile instance
    /// @return The INativeQueryVerifier interface instance
    function getVerifier() internal pure returns (INativeQueryVerifier) {
        return INativeQueryVerifier(PRECOMPILE_ADDRESS);
    }
}
