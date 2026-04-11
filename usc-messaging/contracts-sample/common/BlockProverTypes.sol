// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;


library BlockProverTypes {
    /// @notice Discriminator for the inclusion proof format.
    enum ProofKind {
        /// Binary Merkle tree; `data` is ABI-encoded `MerkleProofEntry[]`.
        BinaryMerkle,
        /// STARK proof; `data` is the raw proof bytes from the prover.
        Stark
    }

    /// @notice A single node in a binary Merkle inclusion proof.
    /// @param sibling Hash of the sibling node at this level.
    /// @param isLeft  True if the sibling is on the left (i.e. the current node is the right child).
    struct MerkleProofEntry {
        bytes32 sibling;
        bool isLeft;
    }

    /// @notice Self-describing transaction-inclusion proof.
    /// @param kind ProofKind discriminator selecting how `data` is interpreted.
    /// @param root Merkle/commitment root of the transaction trie for the target block.
    /// @param data BinaryMerkle: ABI-encoded MerkleProofEntry[].
    ///             Stark:        raw STARK proof bytes.
    struct InclusionProof {
        ProofKind kind;
        bytes32 root;
        bytes data;
    }

    /// @notice Proof that the target block is part of an unbroken, canonical chain.
    ///         Prevents acceptance of proofs built on top of re-org'd blocks.
    ///
    ///         Verification steps performed by the block prover:
    ///           1. Hash each header to derive its block hash.
    ///           2. Extract the parentHash field from each header.
    ///           3. Assert parentHash[i] == keccak256(blockHeaders[i-1]) for every step.
    ///           4. Assert keccak256(blockHeaders[0]) matches a finalized anchor block
    ///              known to the USC attestation chain.
    ///           5. Assert keccak256(blockHeaders[last]) matches the merkle root's block hash.
    ///
    /// @param blockHeaders      Sequential RLP-encoded block headers from the finalized anchor
    ///                          block (inclusive) up to and including the target block.
    /// @param anchorBlockHeight Block height of blockHeaders[0].
    struct ContinuityProof {
        bytes[] blockHeaders;
        uint64 anchorBlockHeight;
    }
}
