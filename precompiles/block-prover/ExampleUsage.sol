// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "../metadata/sol/block_prover.sol";

/// @title SimpleQueryExample
/// @notice A minimal example showing how to use the Native Query Verifier precompile
/// @dev This demonstrates the basic workflow: provide proof data and verify transaction inclusion
contract SimpleQueryExample {
    /// @notice The Native Query Verifier precompile instance
    /// @dev Address: 0x0000000000000000000000000000000000000FD2 (4050 decimal)
    INativeQueryVerifier public immutable verifier;

    /// @notice Emitted when verification succeeds
    event VerificationSuccess(uint64 chainKey, uint64 blockHeight);

    constructor() {
        // Get the precompile instance using the helper library
        verifier = NativeQueryVerifierLib.getVerifier();
    }

    /// @notice Simple example: Verify a transaction is included in a block
    /// @dev This is the most basic usage - verifies transaction inclusion using Merkle proof and continuity chain
    /// @param chainKey The chain key identifier (e.g., 1 for Ethereum mainnet)
    /// @param height The block height to verify
    /// @param encodedTransaction The raw transaction data to verify
    /// @param merkleRoot The Merkle root of the transaction tree
    /// @param siblings Array of Merkle proof siblings with position info
    /// @param lowerEndpointDigest The digest of the block before the continuity chain starts
    /// @param continuityBlocks Array of continuity chain blocks (blocks[0] is at queryHeight-1)
    /// @return success Whether verification succeeded (reverts on failure)
    function verifySimpleQuery(
        uint64 chainKey,
        uint64 height,
        bytes calldata encodedTransaction,
        bytes32 merkleRoot,
        INativeQueryVerifier.MerkleProofEntry[] calldata siblings,
        bytes32 lowerEndpointDigest,
        INativeQueryVerifier.ContinuityBlock[] calldata continuityBlocks
    ) external returns (bool success) {
        // Step 1: Build the Merkle proof
        // The proof proves that txData is included in the block at height
        INativeQueryVerifier.MerkleProof memory merkleProof = INativeQueryVerifier.MerkleProof({
            root: merkleRoot,  // The Merkle root from the block header
            siblings: siblings  // Sibling hashes with position info (isLeft flag)
        });

        // Step 2: Build the continuity proof
        // This proves the block is part of an attested chain
        INativeQueryVerifier.ContinuityProof memory continuityProof = INativeQueryVerifier.ContinuityProof({
            lowerEndpointDigest: lowerEndpointDigest,  // Digest of block before continuity chain
            blocks: continuityBlocks  // Continuity blocks (blocks[0] is at height-1)
        });

        // Step 3: Verify using view function (no events, cheaper gas)
        // This verifies:
        // - The transaction is in the block (Merkle proof)
        // - The block is part of an attested chain (continuity proof)
        // Note: This function reverts on failure, so if it returns, verification succeeded
        bool verified = verifier.verify(
            chainKey,
            height,
            encodedTransaction,
            merkleProof,
            continuityProof
        );

        require(verified, "Verification failed");

        emit VerificationSuccess(chainKey, height);
        return true;
    }

    /// @notice Example: Verify and emit event (state-changing version)
    /// @dev This version emits a TransactionVerified event on success
    /// @param chainKey The chain key identifier
    /// @param height The block height to verify
    /// @param encodedTransaction The raw transaction data to verify
    /// @param merkleRoot The Merkle root
    /// @param siblings Merkle proof siblings
    /// @param lowerEndpointDigest The digest of the block before the continuity chain starts
    /// @param continuityBlocks Continuity chain blocks
    /// @return success Whether verification succeeded (reverts on failure)
    function verifyAndEmitExample(
        uint64 chainKey,
        uint64 height,
        bytes calldata encodedTransaction,
        bytes32 merkleRoot,
        INativeQueryVerifier.MerkleProofEntry[] calldata siblings,
        bytes32 lowerEndpointDigest,
        INativeQueryVerifier.ContinuityBlock[] calldata continuityBlocks
    ) external returns (bool success) {
        // Build Merkle proof
        INativeQueryVerifier.MerkleProof memory merkleProof = INativeQueryVerifier.MerkleProof({
            root: merkleRoot,
            siblings: siblings
        });

        // Build continuity proof
        INativeQueryVerifier.ContinuityProof memory continuityProof = INativeQueryVerifier.ContinuityProof({
            lowerEndpointDigest: lowerEndpointDigest,
            blocks: continuityBlocks
        });

        // Verify and emit event (this emits TransactionVerified event on success)
        // Note: This function reverts on failure, so if it returns, verification succeeded
        bool verified = verifier.verifyAndEmit(
            chainKey,
            height,
            encodedTransaction,
            merkleProof,
            continuityProof
        );

        require(verified, "Verification failed");
        return true;
    }
}

/// @title BatchVerificationExample
/// @notice Example: Verify multiple transactions in a batch
/// @dev Shows how to use batch verification to save gas by verifying continuity chain once
contract BatchVerificationExample {
    INativeQueryVerifier public immutable verifier;

    /// @notice Emitted when batch verification succeeds
    event BatchVerificationSuccess(uint64 chainKey, uint64[] heights);

    constructor() {
        verifier = NativeQueryVerifierLib.getVerifier();
    }

    /// @notice Verify multiple transactions in a single call
    /// @dev Batch verification is more gas-efficient as it verifies the continuity chain once
    /// @param chainKey The chain key identifier (same for all queries)
    /// @param heights Array of block heights to verify
    /// @param encodedTransactions Transaction data for each query
    /// @param merkleProofs Merkle proofs for each query
    /// @param lowerEndpointDigest The digest of the block before the continuity chain starts
    /// @param continuityBlocks Shared continuity chain blocks covering all query heights
    /// @return success Whether all verifications succeeded (reverts on any failure)
    function verifyBatch(
        uint64 chainKey,
        uint64[] calldata heights,
        bytes[] calldata encodedTransactions,
        INativeQueryVerifier.MerkleProof[] calldata merkleProofs,
        bytes32 lowerEndpointDigest,
        INativeQueryVerifier.ContinuityBlock[] calldata continuityBlocks
    ) external returns (bool success) {
        // Build shared continuity proof
        INativeQueryVerifier.ContinuityProof memory continuityProof = INativeQueryVerifier.ContinuityProof({
            lowerEndpointDigest: lowerEndpointDigest,
            blocks: continuityBlocks
        });

        // Verify batch using view function (no events)
        // This verifies all transactions and the shared continuity chain
        // Note: This function reverts on any failure, so if it returns, all verifications succeeded
        bool verified = verifier.verify(
            chainKey,
            heights,
            encodedTransactions,
            merkleProofs,
            continuityProof
        );

        require(verified, "Batch verification failed");

        emit BatchVerificationSuccess(chainKey, heights);
        return true;
    }

    /// @notice Verify multiple transactions and emit events for each
    /// @dev This version emits TransactionVerified event for each successfully verified transaction
    /// @param chainKey The chain key identifier
    /// @param heights Array of block heights to verify
    /// @param encodedTransactions Transaction data for each query
    /// @param merkleProofs Merkle proofs for each query
    /// @param lowerEndpointDigest The digest of the block before the continuity chain starts
    /// @param continuityBlocks Shared continuity chain blocks
    /// @return success Whether all verifications succeeded (reverts on any failure)
    function verifyBatchAndEmit(
        uint64 chainKey,
        uint64[] calldata heights,
        bytes[] calldata encodedTransactions,
        INativeQueryVerifier.MerkleProof[] calldata merkleProofs,
        bytes32 lowerEndpointDigest,
        INativeQueryVerifier.ContinuityBlock[] calldata continuityBlocks
    ) external returns (bool success) {
        // Build shared continuity proof
        INativeQueryVerifier.ContinuityProof memory continuityProof = INativeQueryVerifier.ContinuityProof({
            lowerEndpointDigest: lowerEndpointDigest,
            blocks: continuityBlocks
        });

        // Verify batch and emit events (emits TransactionVerified for each transaction)
        // Note: This function reverts on any failure, so if it returns, all verifications succeeded
        bool verified = verifier.verifyAndEmit(
            chainKey,
            heights,
            encodedTransactions,
            merkleProofs,
            continuityProof
        );

        require(verified, "Batch verification failed");
        return true;
    }
}
