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

    /// @notice Block structure for continuity verification
    /// @dev Represents a block in the continuity chain
    struct Block {
        /// Block number
        uint64 block_number;
        /// Block root hash
        bytes32 root;
        /// Previous block digest
        bytes32 prev_digest;
        /// Current block digest
        bytes32 digest;
    }

    /// @notice Result of query verification
    /// @dev Contains the verification status and extracted data segments
    struct QueryVerificationResult {
        /// Verification status code:
        /// 0 = Success
        /// 1 = MerkleProofInvalid
        /// 2 = ContinuityChainInvalid
        /// 3 = DataExtractionError
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

    /// @notice Verify a blockchain query with Merkle proof and continuity chain
    /// @dev This is a view function that performs native verification at runtime speed
    /// @param query The query specification defining what data to retrieve
    /// @param tx_data Raw transaction data to verify and extract from
    /// @param merkle_proof Merkle proof for transaction inclusion (with position info, no index needed)
    /// @param continuity_blocks Array of blocks for continuity verification
    /// @return result Verification result containing status and extracted data segments
    ///
    /// Gas Costs (aligned with standard Ethereum precompiles):
    /// - Base: 35,000 (reduced for efficiency)
    /// - Per TX byte: 16 (matches EVM calldata cost)
    /// - Per sibling: 3,000 (equal to ecrecover)
    /// - Per continuity block: 5,000
    /// - Storage lookup: 2,600 per attestation/checkpoint (matches cold SLOAD)
    /// - Merkle verification: 100,000 weight
    /// - Continuity verification: 50,000 weight
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
    /// // Verify
    /// INativeQueryVerifier.QueryVerificationResult memory result = verifier.verifyQuery(
    ///     query,
    ///     txData,
    ///     proof,
    ///     blocks
    /// );
    ///
    /// require(result.status == 0, "Verification failed");
    /// // Use result.result_segments...
    /// ```
    function verifyQuery(
        Query calldata query,
        bytes calldata tx_data,
        MerkleProof calldata merkle_proof,
        Block[] calldata continuity_blocks
    ) external view returns (QueryVerificationResult memory result);

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
}

/// @title NativeQueryVerifierLib
/// @notice Helper library for working with the Native Query Verifier precompile
/// @dev Provides convenience functions and constants
library NativeQueryVerifierLib {
    /// @notice Address of the Native Query Verifier precompile
    address constant PRECOMPILE_ADDRESS =
        0x0000000000000000000000000000000000000FD2;

    /// @notice Status code: Verification successful
    uint8 constant STATUS_SUCCESS = 0;
    /// @notice Status code: Merkle proof verification failed
    uint8 constant STATUS_MERKLE_INVALID = 1;
    /// @notice Status code: Continuity chain validation failed
    uint8 constant STATUS_CONTINUITY_INVALID = 2;
    /// @notice Status code: Data extraction error
    uint8 constant STATUS_DATA_ERROR = 3;

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
        return "Unknown error";
    }

    /// @notice Create a simple query for a single data segment
    /// @param chainId The chain identifier
    /// @param height The block height
    /// @param offset The byte offset in the transaction
    /// @param size The number of bytes to extract
    /// @return query The constructed query
    function createSimpleQuery(
        uint64 chainId,
        uint64 height,
        uint64 offset,
        uint64 size
    ) internal pure returns (INativeQueryVerifier.Query memory query) {
        INativeQueryVerifier.LayoutSegment[]
            memory segments = new INativeQueryVerifier.LayoutSegment[](1);
        segments[0] = INativeQueryVerifier.LayoutSegment(offset, size);

        query = INativeQueryVerifier.Query({
            chain_id: chainId,
            height: height,
            layout_segments: segments
        });
    }
}
