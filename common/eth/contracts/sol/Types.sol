// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.0;

// Types, structs, and enums
type QueryId is bytes32;

type Balance is uint256;

struct ChainQuery {
    uint64 chainId;
    uint64 height;
    uint64 index;
    LayoutSegment[] layoutSegments;
}

struct LayoutSegment {
    uint64 offset;
    uint64 size;
}

struct ResultEvidence {
    bytes proof;
    ChainQuery query;
}

struct ResultSegment {
    uint256 offset; // potentially not need due to ordering i
    bytes abiBytes;
}

struct QueryDetails {
    QueryState state;
    ChainQuery query;
    Balance escrowedAmount;
    address principal;
    Balance estimatedCost;
    uint256 timestamp;
}

enum QueryState {
    // ChainQuery is uninitialized, the default state
    Uninitialized,
    // ChainQuery is submitted but not yet verified
    Submitted,
    // ChainQuery is verified and the result is available
    ResultAvailable,
    // ChainQuery targeted a transaction outside of the containing
    // range or the query's layout is impossible given the t
    InvalidQuery
}

enum VerifierExitStatus {
    // Success: proof verifies and requested byte ranges could
    // from the proof.
    Success,
    // ProofInvalid: proof verifier couldn't verify the proof.
    ProofInvalid,
    // LayoutMismatch: CCNode couldn't extract the bytes indic
    // from the submitted proof bytes. (Prover's fault)
    LayoutMismatch,
    // QueryOutOfBounds: the proof shows that either the targe
    // doesn't exist or the query's layout includes segments o
    // targeted transaction. (dApp's fault)
    QueryOutOfBounds
}
