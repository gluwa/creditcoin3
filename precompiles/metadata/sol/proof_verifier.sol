// SPDX-License-Identifier: GPL-3.0-only
pragma solidity >=0.8.3;

/// @dev The precompiled address of the Proof verifier contract on the Ethereum network.
address constant PROOF_VERIFIER_ADDRESS = 0x0000000000000000000000000000000000000Be9;

/// @dev Instance of the QueryVerifierContract interface at the precompiled address.
QueryVerifierContract constant CLAIM_CONTRACT_ADRRESS = QueryVerifierContract(
    PROOF_VERIFIER_ADDRESS
);

struct Query {
    uint64 chainId;
    uint64 height;
    uint64 index;
    LayoutSegment[] layout;
    bytes data;
}

struct LayoutSegment {
    uint64 offset;
    uint64 size;
}

/// @title ProofVerifierContract interface
/// @notice This interface defines the functions and events for interacting with the ProofVerifierContract.
interface QueryVerifierContract {
    /// @notice Submit proof for a claim.
    /// @param proof The proof to be submitted.
    /// @param query The query to be verified.
    function verify(
        bytes calldata proof,
        Query calldata query
    ) external returns (uint64);
}
