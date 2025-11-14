// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "../metadata/sol/INativeQueryVerifier.sol";

/// @title QueryBuilder
/// @notice Helper library for constructing queries
/// @dev Provides convenience functions for creating queries with common patterns
library QueryBuilder {
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

    /// @notice Create a query for ERC20 Transfer event data
    /// @param chainId The chain identifier
    /// @param height The block height
    /// @return query The constructed query for Transfer event (from, to, amount)
    function createERC20TransferQuery(
        uint64 chainId,
        uint64 height
    ) internal pure returns (INativeQueryVerifier.Query memory query) {
        INativeQueryVerifier.LayoutSegment[]
            memory segments = new INativeQueryVerifier.LayoutSegment[](3);
        segments[0] = INativeQueryVerifier.LayoutSegment(192, 32);  // from address
        segments[1] = INativeQueryVerifier.LayoutSegment(224, 32);  // to address
        segments[2] = INativeQueryVerifier.LayoutSegment(256, 32);  // amount

        query = INativeQueryVerifier.Query({
            chain_id: chainId,
            height: height,
            layout_segments: segments
        });
    }
}

/// @title EthereumTransactionVerifier
/// @notice Example contract demonstrating usage of the Native Query Verifier precompile
/// @dev This contract verifies Ethereum transactions and extracts ERC20 transfer data
contract EthereumTransactionVerifier {
    using NativeQueryVerifierLib for INativeQueryVerifier.QueryVerificationResult;

    /// @notice The Native Query Verifier precompile instance
    INativeQueryVerifier public immutable verifier;

    /// @notice Emitted when a transaction is successfully verified
    event TransactionVerified(
        uint64 indexed chainId,
        uint64 indexed blockHeight,
        uint64 indexed txIndex,
        bytes32 queryId
    );

    /// @notice Emitted when verification fails
    event VerificationFailed(
        uint64 indexed chainId,
        uint64 indexed blockHeight,
        uint64 indexed txIndex,
        uint8 status,
        string reason
    );

    constructor() {
        verifier = NativeQueryVerifierLib.getVerifier();
    }

    /// @notice Verify an Ethereum ERC20 transfer transaction
    /// @param blockHeight The Ethereum block height
    /// @param txData The raw transaction data
    /// @param merkleRoot The Merkle root of the transaction tree
    /// @param siblings The Merkle proof siblings
    /// @param blockNumbers The continuity chain block numbers
    /// @param digests The continuity chain digests
    /// @return verified Whether verification succeeded
    /// @return from The from address (if successful)
    /// @return to The to address (if successful)
    /// @return value The transfer amount (if successful)
    function verifySimpleTransfer(
        uint64 blockHeight,
        bytes calldata txData,
        bytes32 merkleRoot,
        bytes32[] calldata siblings,
        uint64[] calldata blockNumbers,
        bytes32[] calldata digests
    ) external returns (
        bool verified,
        address from,
        address to,
        uint256 value
    ) {
        // Create layout segments for ERC20 Transfer event:
        // Topic 0: Transfer(address,address,uint256) - offset 0
        // Topic 1: from address (indexed) - offset 32
        // Topic 2: to address (indexed) - offset 64
        // Data: amount - offset 96
        INativeQueryVerifier.LayoutSegment[] memory segments = new INativeQueryVerifier.LayoutSegment[](3);
        segments[0] = INativeQueryVerifier.LayoutSegment(192, 32);  // from address
        segments[1] = INativeQueryVerifier.LayoutSegment(224, 32);  // to address
        segments[2] = INativeQueryVerifier.LayoutSegment(256, 32);  // amount

        // Create query for Ethereum mainnet
        INativeQueryVerifier.Query memory query = INativeQueryVerifier.Query({
            chain_id: 1,
            height: blockHeight,
            layout_segments: segments
        });

        // Create Merkle proof entries
        INativeQueryVerifier.MerkleProofEntry[] memory proofEntries = new INativeQueryVerifier.MerkleProofEntry[](siblings.length);
        for (uint256 i = 0; i < siblings.length; i++) {
            // Note: isLeft should be determined based on your proof structure
            // This is a simplified example - adjust based on your actual proof format
            proofEntries[i] = INativeQueryVerifier.MerkleProofEntry({
                hash: siblings[i],
                isLeft: false // Adjust based on your proof structure
            });
        }

        INativeQueryVerifier.MerkleProof memory proof = INativeQueryVerifier.MerkleProof({
            root: merkleRoot,
            siblings: proofEntries
        });

        // Create continuity chain blocks
        // Note: This is a simplified example - you need to provide full Block structures
        // with root, prev_digest, and digest fields
        INativeQueryVerifier.Block[] memory continuityBlocks = new INativeQueryVerifier.Block[](blockNumbers.length);
        for (uint256 i = 0; i < blockNumbers.length; i++) {
            continuityBlocks[i] = INativeQueryVerifier.Block({
                block_number: blockNumbers[i],
                root: bytes32(0), // Provide actual root
                prev_digest: i > 0 ? digests[i - 1] : bytes32(0),
                digest: digests[i]
            });
        }

        // Verify query
        INativeQueryVerifier.QueryVerificationResult memory result = verifier.verifyQuery(
            query,
            txData,
            proof,
            continuityBlocks
        );

        // Check result
        if (result.isSuccess()) {
            // Extract data from result segments
            require(result.result_segments.length == 3, "Invalid result segments");

            from = address(uint160(uint256(result.result_segments[0].data)));
            to = address(uint160(uint256(result.result_segments[1].data)));
            value = uint256(result.result_segments[2].data);

            emit TransactionVerified(1, blockHeight, 0, keccak256(abi.encode(query)));
            return (true, from, to, value);
        } else {
            emit VerificationFailed(
                1,
                blockHeight,
                0,
                result.status,
                NativeQueryVerifierLib.getErrorMessage(result.status)
            );
            return (false, address(0), address(0), 0);
        }
    }

    /// @notice Verify a simple Ethereum ETH transfer transaction
    /// @param blockHeight The Ethereum block height
    /// @param txData The raw transaction data
    /// @param merkleRoot The Merkle root
    /// @param siblings The Merkle proof siblings
    /// @param blockNumbers Continuity chain block numbers
    /// @param digests Continuity chain digests
    /// @return success Whether verification succeeded
    /// @return fromAddr The sender address
    /// @return toAddr The recipient address
    /// @return value The transfer value
    function verifyETHTransfer(
        uint64 blockHeight,
        bytes calldata txData,
        bytes32 merkleRoot,
        bytes32[] calldata siblings,
        uint64[] calldata blockNumbers,
        bytes32[] calldata digests
    ) external returns (
        bool success,
        address fromAddr,
        address toAddr,
        uint256 value
    ) {
        // Layout for simple ETH transfer transaction
        // Nonce: 0-8 bytes (RLP encoded)
        // Gas price: variable
        // Gas limit: variable
        // To: 20 bytes at known offset
        // Value: 32 bytes at known offset
        // Data: variable
        INativeQueryVerifier.LayoutSegment[] memory segments = new INativeQueryVerifier.LayoutSegment[](3);
        segments[0] = INativeQueryVerifier.LayoutSegment(0, 32);    // from (derived from signature)
        segments[1] = INativeQueryVerifier.LayoutSegment(64, 32);   // to address
        segments[2] = INativeQueryVerifier.LayoutSegment(96, 32);   // value

        INativeQueryVerifier.Query memory query = INativeQueryVerifier.Query({
            chain_id: 1,
            height: blockHeight,
            layout_segments: segments
        });

        // Create Merkle proof entries
        INativeQueryVerifier.MerkleProofEntry[] memory proofEntries = new INativeQueryVerifier.MerkleProofEntry[](siblings.length);
        for (uint256 i = 0; i < siblings.length; i++) {
            proofEntries[i] = INativeQueryVerifier.MerkleProofEntry({
                hash: siblings[i],
                isLeft: false // Adjust based on your proof structure
            });
        }

        INativeQueryVerifier.MerkleProof memory proof = INativeQueryVerifier.MerkleProof({
            root: merkleRoot,
            siblings: proofEntries
        });

        // Create continuity chain blocks
        INativeQueryVerifier.Block[] memory continuityBlocks = new INativeQueryVerifier.Block[](blockNumbers.length);
        for (uint256 i = 0; i < blockNumbers.length; i++) {
            continuityBlocks[i] = INativeQueryVerifier.Block({
                block_number: blockNumbers[i],
                root: bytes32(0), // Provide actual root
                prev_digest: i > 0 ? digests[i - 1] : bytes32(0),
                digest: digests[i]
            });
        }

        INativeQueryVerifier.QueryVerificationResult memory result = verifier.verifyQuery(
            query,
            txData,
            proof,
            continuityBlocks
        );

        if (result.isSuccess() && result.result_segments.length == 3) {
            fromAddr = address(uint160(uint256(result.result_segments[0].data)));
            toAddr = address(uint160(uint256(result.result_segments[1].data)));
            value = uint256(result.result_segments[2].data);

            emit TransactionVerified(1, blockHeight, 0, keccak256(abi.encode(query)));
            return (true, fromAddr, toAddr, value);
        } else {
            if (!result.isSuccess()) {
                emit VerificationFailed(
                    1,
                    blockHeight,
                    0,
                    result.status,
                    NativeQueryVerifierLib.getErrorMessage(result.status)
                );
            }
            return (false, address(0), address(0), 0);
        }
    }

    /// @notice Verify a transaction with custom layout segments
    /// @param chainId The chain identifier
    /// @param blockHeight The block height
    /// @param txData The raw transaction data
    /// @param segments Custom layout segments to extract
    /// @param merkleProof The Merkle proof for transaction inclusion
    /// @param continuityBlocks The continuity chain blocks
    /// @return result The full verification result
    function verifyCustomQuery(
        uint64 chainId,
        uint64 blockHeight,
        bytes calldata txData,
        INativeQueryVerifier.LayoutSegment[] calldata segments,
        INativeQueryVerifier.MerkleProof calldata merkleProof,
        INativeQueryVerifier.Block[] calldata continuityBlocks
    ) external returns (INativeQueryVerifier.QueryVerificationResult memory result) {
        INativeQueryVerifier.Query memory query = INativeQueryVerifier.Query({
            chain_id: chainId,
            height: blockHeight,
            layout_segments: segments
        });

        result = verifier.verifyQuery(query, txData, merkleProof, continuityBlocks);

        if (result.isSuccess()) {
            emit TransactionVerified(chainId, blockHeight, 0, keccak256(abi.encode(query)));
        } else {
            emit VerificationFailed(
                chainId,
                blockHeight,
                0,
                result.status,
                NativeQueryVerifierLib.getErrorMessage(result.status)
            );
        }

        return result;
    }

    /// @notice Batch verify multiple transactions using shared continuity chain
    /// @dev Uses the optimized batch verification function that verifies continuity once
    /// @param queries Array of queries to verify (max 10)
    /// @param txDataArray Array of transaction data
    /// @param proofs Array of Merkle proofs
    /// @param sharedContinuityBlocks Shared continuity chain covering all query heights
    /// @return batchResult Batch verification result with statistics and individual results
    function batchVerify(
        INativeQueryVerifier.Query[] calldata queries,
        bytes[] calldata txDataArray,
        INativeQueryVerifier.MerkleProof[] calldata proofs,
        INativeQueryVerifier.Block[] calldata sharedContinuityBlocks
    ) external returns (INativeQueryVerifier.BatchQueryVerificationResult memory batchResult) {
        // Use the optimized batch verification function
        batchResult = verifier.verifyBatchQueries(
            queries,
            txDataArray,
            proofs,
            sharedContinuityBlocks
        );

        // Emit events for individual results (batch function already emits events, but
        // we can emit additional custom events if needed)
        for (uint256 i = 0; i < batchResult.results.length; i++) {
            if (batchResult.results[i].isSuccess()) {
                emit TransactionVerified(
                    queries[i].chain_id,
                    queries[i].height,
                    0,
                    keccak256(abi.encode(queries[i]))
                );
            } else {
                emit VerificationFailed(
                    queries[i].chain_id,
                    queries[i].height,
                    0,
                    batchResult.results[i].status,
                    NativeQueryVerifierLib.getErrorMessage(batchResult.results[i].status)
                );
            }
        }

        return batchResult;
    }
}

/// @title CrossChainBridge
/// @notice Example of using the verifier for a cross-chain bridge
/// @dev Verifies deposits on one chain and mints on another.
///      Stores verification results to avoid re-verification if business logic fails.
///      Validates that tokens were burned (sent to burn address) before minting.
contract CrossChainBridge {
    using NativeQueryVerifierLib for INativeQueryVerifier.QueryVerificationResult;

    INativeQueryVerifier public immutable verifier;

    /// @notice The burn address on the source chain
    /// @dev Tokens must be sent to this address to be considered burned
    ///      Example: 0x0000000000000000000000000000000000000001
    address public constant BURN_ADDRESS = address(0x1);

    /// @notice Mapping to track processed transactions
    mapping(bytes32 => bool) public processedTransactions;

    /// @notice Storage for verified query results
    /// @dev Stores verification results so they can be reused if business logic fails
    ///      Key: transaction ID, Value: stored verification result
    mapping(bytes32 => StoredVerificationResult) public verifiedResults;

    /// @notice Stored verification result structure
    /// @dev Stores the verification result and extracted data for reuse
    struct StoredVerificationResult {
        bool verified;
        address depositor;      // from address (who sent the tokens)
        address burnRecipient; // to address (must be burn address)
        uint256 amount;
        uint64 chainId;
        uint64 blockHeight;
        uint256 timestamp;
    }

    event DepositVerified(
        uint64 chainId,
        uint64 blockHeight,
        address depositor,
        uint256 amount
    );

    event VerificationStored(
        bytes32 indexed txId,
        uint64 chainId,
        uint64 blockHeight
    );

    event TokensNotBurned(
        bytes32 indexed txId,
        address recipient,
        address expectedBurnAddress
    );

    constructor() {
        verifier = NativeQueryVerifierLib.getVerifier();
    }

    /// @notice Verify and store a deposit transaction from another chain
    /// @dev This function verifies the transaction and stores the result.
    ///      If verification succeeds, the result is stored for later use.
    ///      This allows retrying business logic without re-verification.
    ///      Validates that tokens were sent to the burn address.
    /// @param chainId The source chain ID
    /// @param blockHeight The block height of the deposit
    /// @param txData The transaction data (ERC20 Transfer event)
    /// @param merkleProof The Merkle proof for transaction inclusion
    /// @param continuityBlocks The continuity chain blocks
    /// @return txId The transaction identifier
    /// @return result The verification result
    function verifyAndStoreDeposit(
        uint64 chainId,
        uint64 blockHeight,
        bytes calldata txData,
        INativeQueryVerifier.MerkleProof calldata merkleProof,
        INativeQueryVerifier.Block[] calldata continuityBlocks
    ) external returns (bytes32 txId, INativeQueryVerifier.QueryVerificationResult memory result) {
        // Create unique identifier for this transaction
        txId = keccak256(abi.encodePacked(chainId, blockHeight, txData));

        // Check if already verified and stored
        StoredVerificationResult storage stored = verifiedResults[txId];
        if (stored.verified) {
            // Return stored result - no need to re-verify
            result.status = 0; // Success
            result.result_segments = new INativeQueryVerifier.ResultSegment[](3);
            result.result_segments[0] = INativeQueryVerifier.ResultSegment({
                offset: 192,
                data: bytes32(uint256(uint160(stored.depositor)))
            });
            result.result_segments[1] = INativeQueryVerifier.ResultSegment({
                offset: 224,
                data: bytes32(uint256(uint160(stored.burnRecipient)))
            });
            result.result_segments[2] = INativeQueryVerifier.ResultSegment({
                offset: 256,
                data: bytes32(stored.amount)
            });
            return (txId, result);
        }

        // Create query for ERC20 Transfer event using helper function
        // Extracts: from address (offset 192), to address (offset 224), amount (offset 256)
        INativeQueryVerifier.Query memory query = QueryBuilder.createERC20TransferQuery(
            chainId,
            blockHeight
        );

        // Verify the transaction (this may emit events)
        result = verifier.verifyQuery(
            query,
            txData,
            merkleProof,
            continuityBlocks
        );

        // Store result if verification succeeded
        if (result.isSuccess() && result.result_segments.length == 3) {
            address depositor = address(uint160(uint256(result.result_segments[0].data)));
            address burnRecipient = address(uint160(uint256(result.result_segments[1].data)));
            uint256 amount = uint256(result.result_segments[2].data);

            // Validate that tokens were sent to the burn address
            require(burnRecipient == BURN_ADDRESS, "Tokens not burned");

            verifiedResults[txId] = StoredVerificationResult({
                verified: true,
                depositor: depositor,
                burnRecipient: burnRecipient,
                amount: amount,
                chainId: chainId,
                blockHeight: blockHeight,
                timestamp: block.timestamp
            });

            emit VerificationStored(txId, chainId, blockHeight);
        }
    }

    /// @notice Process a deposit using stored verification result
    /// @dev This function uses a previously verified result, allowing business logic
    ///      to be retried without re-verification if it fails.
    ///      Validates that tokens were burned before minting.
    /// @param txId The transaction identifier from verifyAndStoreDeposit
    function processDeposit(bytes32 txId) public {
        require(!processedTransactions[txId], "Transaction already processed");

        StoredVerificationResult storage stored = verifiedResults[txId];
        require(stored.verified, "Transaction not verified");

        // Double-check that tokens were sent to burn address (defense in depth)
        require(stored.burnRecipient == BURN_ADDRESS, "Tokens not burned");

        // Mark as processed BEFORE executing business logic
        // This prevents re-entry but note: if business logic fails, the transaction
        // will be marked as processed. Consider using a two-phase commit pattern.
        processedTransactions[txId] = true;

        // Execute business logic (e.g., mint tokens)
        // Only mint if tokens were verified to be burned on source chain
        // If this fails, the transaction is already marked as processed,
        // but the verification result remains stored for reference
        // _mint(stored.depositor, stored.amount);

        emit DepositVerified(
            stored.chainId,
            stored.blockHeight,
            stored.depositor,
            stored.amount
        );
    }

    /// @notice Process a deposit in a single transaction (verify + process)
    /// @dev This combines verification and processing. If business logic fails,
    ///      the verification result is still stored for retry via processDeposit().
    /// @param chainId The source chain ID
    /// @param blockHeight The block height of the deposit
    /// @param txData The transaction data
    /// @param merkleProof The Merkle proof for transaction inclusion
    /// @param continuityBlocks The continuity chain blocks
    function verifyAndProcessDeposit(
        uint64 chainId,
        uint64 blockHeight,
        bytes calldata txData,
        INativeQueryVerifier.MerkleProof calldata merkleProof,
        INativeQueryVerifier.Block[] calldata continuityBlocks
    ) external {
        // Verify and store (reuses stored result if already verified)
        (bytes32 txId, INativeQueryVerifier.QueryVerificationResult memory result) = 
            this.verifyAndStoreDeposit(chainId, blockHeight, txData, merkleProof, continuityBlocks);

        require(result.isSuccess(), NativeQueryVerifierLib.getErrorMessage(result.status));

        // Process using stored result
        processDeposit(txId);
    }
}
