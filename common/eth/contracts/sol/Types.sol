// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.24;

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
    bytes32 abiBytes;
}

struct QueryDetails {
    QueryState state;
    ChainQuery query;
    Balance escrowedAmount;
    address principal;
    Balance estimatedCost;
    uint256 timestamp;
    ResultSegment[] resultSegments;
}

enum QueryState {
    // ChainQuery is uninitialized, the default state
    Uninitialized,
    // ChainQuery is submitted but not yet verified
    Submitted,
    // ChainQuery is verified and the result is available
    ResultAvailable,
    // The query is malformed such that proving is not possible.
    // This can happen when the Query targeted a transaction number 
    // not contained in the queried block, or the query's layout calls 
    // for data beyond the range of bytes present in the transaction.
    // This state doesn't allow for retries.
    InvalidQuery,
    // There are many reasons why the prover might fail to process
    // a query, unrelated to that query's validity. The prover
    // might be be misconfigured, or networking could fail. We allow
    // for retries in these cases.
    QueryProcessingFailed
}

enum VerifierExitStatus {
    // Success: proof verifies and requested byte ranges could
    // from the proof.
    Success
    // note: since https://github.com/gluwa/creditcoin3-next/pull/608
    // non Success statuses are no longer returned, instead the precompile reverts
}

struct VerifierResult {
    VerifierExitStatus status;
    ResultSegment[] resultSegments;
}
