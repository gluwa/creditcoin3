// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "./INativeQueryVerifier.sol";

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
    /// @param txIndex The transaction index in the block
    /// @param txData The raw transaction data
    /// @param merkleRoot The Merkle root of the transaction tree
    /// @param siblings The Merkle proof siblings
    /// @param blockNumbers The continuity chain block numbers
    /// @param digests The continuity chain digests
    /// @return success Whether verification succeeded
    /// @return from The from address (if successful)
    /// @return to The to address (if successful)
    /// @return amount The transfer amount (if successful)
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

        // Create Merkle proof
        INativeQueryVerifier.MerkleProof memory proof = INativeQueryVerifier.MerkleProof({
            root: merkleRoot,
            siblings: siblings
        });

        // Create continuity chain
        INativeQueryVerifier.ContinuityChain memory continuity = INativeQueryVerifier.ContinuityChain({
            block_numbers: blockNumbers,
            digests: digests
        });

        // Verify query
        INativeQueryVerifier.QueryVerificationResult memory result = verifier.verifyQuery(
            query,
            txData,
            proof,
            continuity
        );

        // Check result
        if (result.isSuccess()) {
            // Extract data from result segments
            require(result.result_segments.length == 3, "Invalid result segments");

            from = address(uint160(uint256(result.result_segments[0].bytes)));
            to = address(uint160(uint256(result.result_segments[1].bytes)));
            amount = uint256(result.result_segments[2].bytes);

            emit TransactionVerified(1, blockHeight, txIndex, keccak256(abi.encode(query)));
            return (true, from, to, amount);
        } else {
            emit VerificationFailed(
                1,
                blockHeight,
                txIndex,
                result.status,
                NativeQueryVerifierLib.getErrorMessage(result.status)
            );
            return (false, address(0), address(0), 0);
        }
    }

    /// @notice Verify a simple Ethereum transaction (value transfer)
    /// @param blockHeight The Ethereum block height
    /// @param txIndex The transaction index in the block
    /// @param txData The raw transaction data
    /// @param merkleRoot The Merkle root
    /// @param siblings The Merkle proof siblings
    /// @param blockNumbers Continuity chain block numbers
    /// @param digests Continuity chain digests
    /// @return success Whether verification succeeded
    /// @return fromAddr The sender address
    /// @return toAddr The recipient address
    /// @return value The transfer value
    function verifySimpleTransfer(
        uint64 blockHeight,
        uint64 txIndex,
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

        INativeQueryVerifier.MerkleProof memory proof = INativeQueryVerifier.MerkleProof({
            root: merkleRoot,
            siblings: siblings
        });

        INativeQueryVerifier.ContinuityChain memory continuity = INativeQueryVerifier.ContinuityChain({
            block_numbers: blockNumbers,
            digests: digests
        });

        INativeQueryVerifier.QueryVerificationResult memory result = verifier.verifyQuery(
            query,
            txData,
            proof,
            continuity
        );

        if (result.isSuccess() && result.result_segments.length == 3) {
            fromAddr = address(uint160(uint256(result.result_segments[0].bytes)));
            toAddr = address(uint160(uint256(result.result_segments[1].bytes)));
            value = uint256(result.result_segments[2].bytes);

            emit TransactionVerified(1, blockHeight, txIndex, keccak256(abi.encode(query)));
            return (true, fromAddr, toAddr, value);
        } else {
            if (!result.isSuccess()) {
                emit VerificationFailed(
                    1,
                    blockHeight,
                    txIndex,
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
    /// @param txIndex The transaction index
    /// @param txData The raw transaction data
    /// @param segments Custom layout segments to extract
    /// @param merkleRoot The Merkle root
    /// @param siblings The Merkle proof siblings
    /// @param blockNumbers Continuity chain block numbers
    /// @param digests Continuity chain digests
    /// @return result The full verification result
    function verifyCustomQuery(
        uint64 chainId,
        uint64 blockHeight,
        bytes calldata txData,
        INativeQueryVerifier.LayoutSegment[] calldata segments,
        bytes32 merkleRoot,
        bytes32[] calldata siblings,
        uint64[] calldata blockNumbers,
        bytes32[] calldata digests
    ) external returns (INativeQueryVerifier.QueryVerificationResult memory result) {
        INativeQueryVerifier.Query memory query = INativeQueryVerifier.Query({
            chain_id: chainId,
            height: blockHeight,
            layout_segments: segments
        });

        INativeQueryVerifier.MerkleProof memory proof = INativeQueryVerifier.MerkleProof({
            root: merkleRoot,
            siblings: siblings
        });

        INativeQueryVerifier.ContinuityChain memory continuity = INativeQueryVerifier.ContinuityChain({
            block_numbers: blockNumbers,
            digests: digests
        });

        result = verifier.verifyQuery(query, txData, proof, continuity);

        if (result.isSuccess()) {
            emit TransactionVerified(chainId, blockHeight, txIndex, keccak256(abi.encode(query)));
        } else {
            emit VerificationFailed(
                chainId,
                blockHeight,
                txIndex,
                result.status,
                NativeQueryVerifierLib.getErrorMessage(result.status)
            );
        }

        return result;
    }

    /// @notice Batch verify multiple transactions
    /// @param queries Array of queries to verify
    /// @param txDataArray Array of transaction data
    /// @param proofs Array of Merkle proofs
    /// @param continuityChains Array of continuity chains
    /// @return results Array of verification results
    function batchVerify(
        INativeQueryVerifier.Query[] calldata queries,
        bytes[] calldata txDataArray,
        INativeQueryVerifier.MerkleProof[] calldata proofs,
        INativeQueryVerifier.ContinuityChain[] calldata continuityChains
    ) external returns (INativeQueryVerifier.QueryVerificationResult[] memory results) {
        require(
            queries.length == txDataArray.length &&
            queries.length == proofs.length &&
            queries.length == continuityChains.length,
            "Array length mismatch"
        );

        results = new INativeQueryVerifier.QueryVerificationResult[](queries.length);

        for (uint256 i = 0; i < queries.length; i++) {
            results[i] = verifier.verifyQuery(
                queries[i],
                txDataArray[i],
                proofs[i],
                continuityChains[i]
            );

            if (results[i].isSuccess()) {
                emit TransactionVerified(
                    queries[i].chain_id,
                    queries[i].height,
                    queries[i].index,
                    keccak256(abi.encode(queries[i]))
                );
            } else {
                emit VerificationFailed(
                    queries[i].chain_id,
                    queries[i].height,
                    queries[i].index,
                    results[i].status,
                    NativeQueryVerifierLib.getErrorMessage(results[i].status)
                );
            }
        }

        return results;
    }
}

/// @title CrossChainBridge
/// @notice Example of using the verifier for a cross-chain bridge
/// @dev Verifies deposits on one chain and mints on another
contract CrossChainBridge {
    using NativeQueryVerifierLib for INativeQueryVerifier.QueryVerificationResult;

    INativeQueryVerifier public immutable verifier;

    mapping(bytes32 => bool) public processedTransactions;

    event DepositVerified(
        uint64 chainId,
        uint64 blockHeight,
        uint64 txIndex,
        address depositor,
        uint256 amount
    );

    constructor() {
        verifier = NativeQueryVerifierLib.getVerifier();
    }

    /// @notice Process a deposit from another chain
    /// @param chainId The source chain ID
    /// @param blockHeight The block height of the deposit
    /// @param txIndex The transaction index
    /// @param txData The transaction data
    /// @param merkleRoot The Merkle root
    /// @param siblings Merkle proof siblings
    /// @param blockNumbers Continuity chain block numbers
    /// @param digests Continuity chain digests
    function processDeposit(
        uint64 chainId,
        uint64 blockHeight,
        uint64 txIndex,
        bytes calldata txData,
        bytes32 merkleRoot,
        bytes32[] calldata siblings,
        uint64[] calldata blockNumbers,
        bytes32[] calldata digests
    ) external {
        // Create unique identifier for this transaction
        bytes32 txId = keccak256(abi.encodePacked(chainId, blockHeight, txIndex));
        require(!processedTransactions[txId], "Transaction already processed");

        // Define layout for deposit event
        INativeQueryVerifier.LayoutSegment[] memory segments = new INativeQueryVerifier.LayoutSegment[](2);
        segments[0] = INativeQueryVerifier.LayoutSegment(192, 32);  // depositor address
        segments[1] = INativeQueryVerifier.LayoutSegment(224, 32);  // amount

        INativeQueryVerifier.Query memory query = INativeQueryVerifier.Query({
            chain_id: chainId,
            height: blockHeight,
            index: txIndex,
            layout_segments: segments
        });

        INativeQueryVerifier.MerkleProof memory proof = INativeQueryVerifier.MerkleProof({
            root: merkleRoot,
            siblings: siblings
        });

        INativeQueryVerifier.ContinuityChain memory continuity = INativeQueryVerifier.ContinuityChain({
            block_numbers: blockNumbers,
            digests: digests
        });

        // Verify the transaction
        INativeQueryVerifier.QueryVerificationResult memory result = verifier.verifyQuery(
            query,
            txData,
            proof,
            continuity
        );

        require(result.isSuccess(), NativeQueryVerifierLib.getErrorMessage(result.status));
        require(result.result_segments.length == 2, "Invalid result segments");

        // Extract deposit information
        address depositor = address(uint160(uint256(result.result_segments[0].bytes)));
        uint256 amount = uint256(result.result_segments[1].bytes);

        // Mark as processed
        processedTransactions[txId] = true;

        // Mint tokens or perform other actions
        // _mint(depositor, amount);

        emit DepositVerified(chainId, blockHeight, txIndex, depositor, amount);
    }
}
