// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title INativeQueryVerifier
/// @notice Block-prover precompile at `0x…0FD2` (4050): native verification of a transaction's
/// inclusion in a finalized block of an attested chain, via a Merkle proof + continuity chain.
/// @dev Lean vendored copy of `precompiles/metadata/sol/block_prover.sol` — only the structs and
/// the single-query view `verify` used by the write-ability AcknowledgmentValidator. Keep the
/// struct layouts + signature byte-identical with the canonical interface in that file.
interface INativeQueryVerifier {
    struct MerkleProofEntry {
        bytes32 hash;
        bool isLeft;
    }

    struct MerkleProof {
        bytes32 root;
        MerkleProofEntry[] siblings;
    }

    struct ContinuityProof {
        bytes32 lowerEndpointDigest;
        bytes32[] roots;
    }

    /// @notice Verify a transaction's inclusion in a finalized block (read-only). Reverts on
    /// failure, returns true on success.
    function verify(
        uint64 chainKey,
        uint64 height,
        bytes calldata encodedTransaction,
        MerkleProof calldata merkleProof,
        ContinuityProof calldata continuityProof
    ) external view returns (bool);
}

/// @notice Helper for the Native Query Verifier precompile.
library NativeQueryVerifierLib {
    address constant PRECOMPILE_ADDRESS = 0x0000000000000000000000000000000000000FD2;

    function getVerifier() internal pure returns (INativeQueryVerifier) {
        return INativeQueryVerifier(PRECOMPILE_ADDRESS);
    }
}
