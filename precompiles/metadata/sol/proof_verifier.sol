// SPDX-License-Identifier: GPL-3.0-only
pragma solidity >=0.8.3;

/// @dev The precompiled address of the Proof verifier contract on the Ethereum network.
address constant PROOF_VERIFIER_ADDRESS = 0x0000000000000000000000000000000000000Be9;

/// @dev Instance of the QueryVerifierContract interface at the precompiled address.
QueryVerifierContract constant CLAIM_CONTRACT_ADRRESS = QueryVerifierContract(
    PROOF_VERIFIER_ADDRESS
);

type QueryId is bytes32;

struct Query {
    uint64 chainId;
    uint64 height;
    uint64 index;
    LayoutSegment[] layout;
}

struct LayoutSegment {
    uint64 offset;
    uint64 size;
}

struct ResultSegment {
    uint256 offset; // potentially not need due to ordering i
    bytes abiBytes;
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

    /// @notice Retrieve result segments for the given query_id if present in pallet-prover
    /// @param queryId, the id of the query for which we are retrieving result segments
    function get_result_segments(
        QueryId queryId
    ) external returns (ResultSegment[] memory);
}
