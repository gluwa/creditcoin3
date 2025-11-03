// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title INativeQueryVerifier
/// @notice Interface for the Native Query Verifier Precompile at address 0x0FD2 (4050)
/// @dev This precompile provides native-speed verification of blockchain queries using
///      Merkle proofs and continuity chains.
interface INativeQueryVerifier {
    /// @notice Query structure defining what data to retrieve from a blockchain
    /// @dev Specifies the chain, block, transaction, and data segments to extract
    struct Query {
        /// Chain identifier (e.g., 1 for Ethereum mainnet)
        uint64 chain_id;
        /// Block height/number
        uint64 height;
        /// Transaction index in the block
        uint64 index;
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

    /// @notice Merkle proof for verifying transaction inclusion in a block
    /// @dev Contains the Merkle root and sibling hashes needed for verification
    struct MerkleProof {
        /// The Merkle root hash of the transaction tree
        bytes32 root;
        /// Sibling hashes for the Merkle proof path
        bytes32[] siblings;
    }

    /// @notice Continuity chain for verifying block attestations
    /// @dev Links a sequence of blocks through attestations or checkpoints
    struct ContinuityChain {
        /// Block numbers in the continuity chain
        uint64[] block_numbers;
        /// Block digests (hashes) corresponding to each block number
        bytes32[] digests;
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
        bytes32 bytes;
    }

    /// @notice Verify a blockchain query with Merkle proof and continuity chain
    /// @dev This is a view function that performs native verification at runtime speed
    /// @param query The query specification defining what data to retrieve
    /// @param tx_data Raw transaction data to verify and extract from
    /// @param merkle_proof Merkle proof for transaction inclusion in the block
    /// @param continuity_chain Chain of block attestations for continuity verification
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
    /// INativeQueryVerifier verifier = INativeQueryVerifier(0x0000000000000000000000000000000000000BEA);
    ///
    /// // Define data segments to extract (e.g., ERC20 transfer event)
    /// INativeQueryVerifier.LayoutSegment[] memory segments = new INativeQueryVerifier.LayoutSegment[](2);
    /// segments[0] = INativeQueryVerifier.LayoutSegment(192, 32);  // from address
    /// segments[1] = INativeQueryVerifier.LayoutSegment(224, 32);  // to address
    ///
    /// // Create query
    /// INativeQueryVerifier.Query memory query = INativeQueryVerifier.Query({
    ///     chain_id: 1,
    ///     height: 18000000,
    ///     index: 42,
    ///     layout_segments: segments
    /// });
    ///
    /// // Verify
    /// INativeQueryVerifier.QueryVerificationResult memory result = verifier.verifyQuery(
    ///     query,
    ///     txData,
    ///     proof,
    ///     continuity
    /// );
    ///
    /// require(result.status == 0, "Verification failed");
    /// // Use result.result_segments...
    /// ```
    function verifyQuery(
        Query calldata query,
        bytes calldata tx_data,
        MerkleProof calldata merkle_proof,
        ContinuityChain calldata continuity_chain
    ) external view returns (QueryVerificationResult memory result);
}

/// @title NativeQueryVerifierLib
/// @notice Helper library for working with the Native Query Verifier precompile
/// @dev Provides convenience functions and constants
library NativeQueryVerifierLib {
    /// @notice Address of the Native Query Verifier precompile
    address constant PRECOMPILE_ADDRESS = 0x0000000000000000000000000000000000000BEA;

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
    function isSuccess(INativeQueryVerifier.QueryVerificationResult memory result) internal pure returns (bool) {
        return result.status == STATUS_SUCCESS;
    }

    /// @notice Get a human-readable error message for a status code
    /// @param status The status code
    /// @return Error message string
    function getErrorMessage(uint8 status) internal pure returns (string memory) {
        if (status == STATUS_SUCCESS) return "Success";
        if (status == STATUS_MERKLE_INVALID) return "Merkle proof invalid";
        if (status == STATUS_CONTINUITY_INVALID) return "Continuity chain invalid";
        if (status == STATUS_DATA_ERROR) return "Data extraction error";
        return "Unknown error";
    }

    /// @notice Create a simple query for a single data segment
    /// @param chainId The chain identifier
    /// @param height The block height
    /// @param index The transaction index
    /// @param offset The byte offset in the transaction
    /// @param size The number of bytes to extract
    /// @return query The constructed query
    function createSimpleQuery(
        uint64 chainId,
        uint64 height,
        uint64 index,
        uint64 offset,
        uint64 size
    ) internal pure returns (INativeQueryVerifier.Query memory query) {
        INativeQueryVerifier.LayoutSegment[] memory segments = new INativeQueryVerifier.LayoutSegment[](1);
        segments[0] = INativeQueryVerifier.LayoutSegment(offset, size);

        query = INativeQueryVerifier.Query({
            chain_id: chainId,
            height: height,
            index: index,
            layout_segments: segments
        });
    }
}
