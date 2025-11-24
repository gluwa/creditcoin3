// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "../metadata/sol/INativeQueryVerifier.sol";

/// @title SimpleQueryExample
/// @notice A minimal example showing how to use the Native Query Verifier precompile
/// @dev This demonstrates the basic workflow: create a query, provide proof data, and verify
contract SimpleQueryExample {
    /// @notice The Native Query Verifier precompile instance
    /// @dev Address: 0x0000000000000000000000000000000000000FD2 (4050 decimal)
    INativeQueryVerifier public immutable verifier;

    /// @notice Emitted when verification succeeds
    event VerificationSuccess(uint64 chainId, uint64 blockHeight, bytes32 extractedData);

    /// @notice Emitted when verification fails
    event VerificationFailed(uint8 status, string reason);

    constructor() {
        // Get the precompile instance using the helper library
        verifier = NativeQueryVerifierLib.getVerifier();
    }

    /// @notice Simple example: Verify a query and extract a single 32-byte value
    /// @dev This is the most basic usage - extracts one data segment from a transaction
    /// @param chainId The chain identifier (e.g., 1 for Ethereum mainnet)
    /// @param blockHeight The block height to query
    /// @param txData The raw transaction data
    /// @param merkleRoot The Merkle root of the transaction tree
    /// @param siblings Array of Merkle proof siblings with position info
    /// @param continuityBlocks Array of continuity chain blocks (must include queryHeight-1 and queryHeight)
    /// @param offset Byte offset in transaction data to extract from
    /// @return success Whether verification succeeded
    /// @return extractedData The extracted 32-byte value
    function verifySimpleQuery(
        uint64 chainId,
        uint64 blockHeight,
        bytes calldata txData,
        bytes32 merkleRoot,
        INativeQueryVerifier.MerkleProofEntry[] calldata siblings,
        INativeQueryVerifier.Block[] calldata continuityBlocks,
        uint64 offset
    ) external returns (bool success, bytes32 extractedData) {
        // Step 1: Create a query specifying what data to extract
        // We want to extract 32 bytes starting at the given offset
        INativeQueryVerifier.LayoutSegment[] memory segments = new INativeQueryVerifier.LayoutSegment[](1);
        segments[0] = INativeQueryVerifier.LayoutSegment({
            offset: offset,  // Where to start reading in txData
            size: 32          // How many bytes to extract (must be 32 for bytes32)
        });

        INativeQueryVerifier.Query memory query = INativeQueryVerifier.Query({
            chain_id: chainId,
            height: blockHeight,
            layout_segments: segments
        });

        // Step 2: Build the Merkle proof
        // The proof proves that txData is included in the block at blockHeight
        INativeQueryVerifier.MerkleProof memory proof = INativeQueryVerifier.MerkleProof({
            root: merkleRoot,  // The Merkle root from the block header
            siblings: siblings  // Sibling hashes with position info (isLeft flag)
        });

        // Step 3: Verify the query using the precompile
        // This verifies:
        // - The transaction is in the block (Merkle proof)
        // - The block is part of an attested chain (continuity proof)
        // - Extracts the requested data segments
        // Note: This function reverts on failure, so if it returns, verification succeeded
        INativeQueryVerifier.ResultSegment[] memory segments = verifier.verifyQuery(
            query,
            txData,
            proof,
            continuityBlocks
        );

        // Step 4: Extract the data (function already verified successfully)
        require(segments.length == 1, "Invalid result segments");
        extractedData = segments[0].data;
        
        emit VerificationSuccess(chainId, blockHeight, extractedData);
        return (true, extractedData);
    }

    /// @notice Example: Extract multiple values from a transaction (e.g., ERC20 Transfer event)
    /// @dev This shows how to extract multiple 32-byte segments from transaction data
    /// @param chainId The chain identifier
    /// @param blockHeight The block height
    /// @param txData The raw transaction data
    /// @param merkleRoot The Merkle root
    /// @param siblings Merkle proof siblings
    /// @param continuityBlocks Continuity chain blocks
    /// @return success Whether verification succeeded
    /// @return value1 First extracted value (e.g., from address)
    /// @return value2 Second extracted value (e.g., to address)
    /// @return value3 Third extracted value (e.g., amount)
    function verifyMultiSegmentQuery(
        uint64 chainId,
        uint64 blockHeight,
        bytes calldata txData,
        bytes32 merkleRoot,
        INativeQueryVerifier.MerkleProofEntry[] calldata siblings,
        INativeQueryVerifier.Block[] calldata continuityBlocks
    ) external returns (
        bool success,
        bytes32 value1,
        bytes32 value2,
        bytes32 value3
    ) {
        // Create query with 3 segments (e.g., ERC20 Transfer: from, to, amount)
        INativeQueryVerifier.LayoutSegment[] memory segments = new INativeQueryVerifier.LayoutSegment[](3);
        segments[0] = INativeQueryVerifier.LayoutSegment(192, 32);  // First value at offset 192
        segments[1] = INativeQueryVerifier.LayoutSegment(224, 32);  // Second value at offset 224
        segments[2] = INativeQueryVerifier.LayoutSegment(256, 32);  // Third value at offset 256

        INativeQueryVerifier.Query memory query = INativeQueryVerifier.Query({
            chain_id: chainId,
            height: blockHeight,
            layout_segments: segments
        });

        INativeQueryVerifier.MerkleProof memory proof = INativeQueryVerifier.MerkleProof({
            root: merkleRoot,
            siblings: siblings
        });

        // Verify using view function (no events, cheaper gas)
        // Note: This function reverts on failure, so if it returns, verification succeeded
        INativeQueryVerifier.ResultSegment[] memory segments = verifier.verifyQueryView(
            query,
            txData,
            proof,
            continuityBlocks
        );

        require(segments.length == 3, "Invalid result segments");
        value1 = segments[0].data;
        value2 = segments[1].data;
        value3 = segments[2].data;
        return (true, value1, value2, value3);
    }
}

/// @title ERC20TransferVerifier
/// @notice Practical example: Verify an ERC20 Transfer event and extract transfer details
/// @dev Shows real-world usage for cross-chain bridges or token tracking
contract ERC20TransferVerifier {
    INativeQueryVerifier public immutable verifier;

    /// @notice Transfer details extracted from verified transaction
    struct TransferDetails {
        address from;
        address to;
        uint256 amount;
        uint64 chainId;
        uint64 blockHeight;
    }

    event TransferVerified(TransferDetails transfer);

    constructor() {
        verifier = NativeQueryVerifierLib.getVerifier();
    }

    /// @notice Verify an ERC20 Transfer event and extract transfer details
    /// @dev ERC20 Transfer event layout:
    ///      - Topic 0: Transfer(address,address,uint256) signature
    ///      - Topic 1: from address (indexed) - offset 192
    ///      - Topic 2: to address (indexed) - offset 224
    ///      - Data: amount - offset 256
    /// @param chainId Source chain ID (e.g., 1 for Ethereum)
    /// @param blockHeight Block height containing the transfer
    /// @param txData The transaction data containing the Transfer event
    /// @param merkleRoot Merkle root proving transaction inclusion
    /// @param siblings Merkle proof siblings with position info
    /// @param continuityBlocks Continuity chain blocks
    /// @return transfer The verified transfer details
    function verifyERC20Transfer(
        uint64 chainId,
        uint64 blockHeight,
        bytes calldata txData,
        bytes32 merkleRoot,
        INativeQueryVerifier.MerkleProofEntry[] calldata siblings,
        INativeQueryVerifier.Block[] calldata continuityBlocks
    ) external returns (TransferDetails memory transfer) {
        // Define segments to extract from Transfer event
        INativeQueryVerifier.LayoutSegment[] memory segments = new INativeQueryVerifier.LayoutSegment[](3);
        segments[0] = INativeQueryVerifier.LayoutSegment(192, 32);  // from address
        segments[1] = INativeQueryVerifier.LayoutSegment(224, 32);  // to address
        segments[2] = INativeQueryVerifier.LayoutSegment(256, 32);  // amount

        INativeQueryVerifier.Query memory query = INativeQueryVerifier.Query({
            chain_id: chainId,
            height: blockHeight,
            layout_segments: segments
        });

        INativeQueryVerifier.MerkleProof memory proof = INativeQueryVerifier.MerkleProof({
            root: merkleRoot,
            siblings: siblings
        });

        // Verify the query
        // Note: This function reverts on failure, so if it returns, verification succeeded
        INativeQueryVerifier.ResultSegment[] memory segments = verifier.verifyQuery(
            query,
            txData,
            proof,
            continuityBlocks
        );

        require(segments.length == 3, "Invalid result segments");

        // Extract and convert to Solidity types
        transfer = TransferDetails({
            from: address(uint160(uint256(segments[0].data))),
            to: address(uint160(uint256(segments[1].data))),
            amount: uint256(segments[2].data),
            chainId: chainId,
            blockHeight: blockHeight
        });

        emit TransferVerified(transfer);
    }
}
