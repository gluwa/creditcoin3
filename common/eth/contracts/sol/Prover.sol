// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.0;

import './Types.sol';
import './Ownable.sol';

address constant PROOF_VERIFIER_ADDRESS = 0x0000000000000000000000000000000000000Be9;

contract CreditcoinPublicProver is Ownable {
    mapping(QueryId => QueryDetails) public queries;
    QueryId[] public queryIds;
    Balance totalEscrowBalance;
    QueryVerifierContract verifier;
    address proceedsAccount;

    constructor(address _proceedsAccount) Ownable() {
        verifier = QueryVerifierContract(PROOF_VERIFIER_ADDRESS);
        proceedsAccount = _proceedsAccount;
    }

    function computeQueryCost(Query calldata /*query*/) public pure returns (uint256) {
        // An arbitrary cost function defined by the prover.
        // The cost may be a function of the query's target height,
        // the number of segments, the number of bytes in each segment, etc.
        return 1337;
    }

    function submitQuery(Query calldata query, address principal) public payable {
        // Guards
        // QueryId may be computed differently if you'd like.
        QueryId queryId = QueryId.wrap(keccak256(abi.encode(query)));
        // require(queries[queryId].principal == address(0));
        // Need a more complex guard for the queries that allows replay attack protection.

        uint256 estimatedCost = computeQueryCost(query);
        require(msg.value > estimatedCost);

        // Store query details
        // .state
        queries[queryId].state = QueryState.Submitted;
        // .query
        queries[queryId].query.chainId = query.chainId;
        queries[queryId].query.height = query.height;
        queries[queryId].query.index = query.index;
        for (uint i = 0; i < query.layoutSegments.length; i++) {
            queries[queryId].query.layoutSegments.push(query.layoutSegments[i]);
        }
        // .result doesn't need to be set here
        // .escrowedAmount
        queries[queryId].escrowedAmount = Balance.wrap(msg.value);
        // .principal
        queries[queryId].principal = principal;
        // .estimatedCost
        queries[queryId].estimatedCost = Balance.wrap(estimatedCost);
        // .timestamp
        queries[queryId].timestamp = block.number;

        // Add to unprocessed queries
        queryIds.push(queryId);

        // Emit event
        emit QuerySubmitted(queryId, estimatedCost, msg.value, query);
    }

    function reclaimEscrowedPayment(QueryId queryId) public {
        // requires guards for correct query state and timeout
        // -- the timeout criteria may need a more complex helper function
        //    to compute the "deadline".
        //    it may be a function of the distance between the target height
        //    (using some conversion factor in the chain registry) and the current
        //    block number plus the expected compute delay for the
        //    proof generation.
        // allows the dApp to reclaim the escrowed payment if the prover fails
        // to submit a proof or fails otherwise
        // the escrowed payment is transferred to the principal specified
        // in the submitQuery call
        require(queries[queryId].principal == msg.sender);

        QueryState state = queries[queryId].state;
        require(state == QueryState.TimedOut || state == QueryState.InvalidQuery);

        uint256 escrowedAmount = Balance.unwrap(queries[queryId].escrowedAmount);
        (bool sent, ) = msg.sender.call{value: escrowedAmount}("");
        require(sent, "Failed to send Fee");

        emit EscrowedPaymentReclaimed(queryId, escrowedAmount);
    }

    // submitQueryProof is called by the prover when a query's proof is ready.
    function submitQueryProof(QueryId queryId, bytes calldata proof) public onlyOwner returns (uint64) {
        // Fist verify the proof
        uint64 result = verifier.verify(proof, queries[queryId].query);

        // Calculate the prover's fee
        // Transfer the prover's fee to the prover
        uint256 proverFee = Balance.unwrap(queries[queryId].escrowedAmount);
        // Send to proceedsAccount
        (bool sent, ) = proceedsAccount.call{value: proverFee}("");
        require(sent, "Failed to send Fee");

        // Check the result of the proof verification
        // After the fee is processed, the state of the query should be updated
        if (result == 0) {
            queries[queryId].state = QueryState.ResultAvailable;
        } else if (result == 1) {
            queries[queryId].state = QueryState.InvalidQuery;
            revert("LayoutMismatch");
        } else if (result == 2) {
            queries[queryId].state = QueryState.TimedOut;
            revert("ProofInvalid");
        } else if (result == 3) {
            queries[queryId].state = QueryState.InvalidQuery;
            revert("QueryOutOfBounds");
        }

        emit QueryProofVerified(queryId, proof);

        // Clean up the processed queryId from the array
        _removeQueryId(queryId);

        return result;
    }

    function withdrawProceeds() public onlyOwner {
        // requires owner guard
        // allows the prover to withdraw the balance of the contract that's not
        // still escrowed
    }

    function getUnprocessedQueries() public view returns (Query[] memory) {
        uint256 unprocessedCount;

        for (uint256 i = 0; i < queryIds.length; i++) {
            if (queries[queryIds[i]].state == QueryState.Submitted) {
                unprocessedCount++;
            }
        }

        Query[] memory unprocessedQueries = new Query[](unprocessedCount);
        uint256 index;

        for (uint256 i = 0; i < queryIds.length; i++) {
            if (queries[queryIds[i]].state == QueryState.Submitted) {
                unprocessedQueries[index] = queries[queryIds[i]].query;
                index++;
            }
        }

        return unprocessedQueries;
    }

    function _removeQueryId(QueryId queryId) private {
        uint256 length = queryIds.length;
        for (uint256 i = 0; i < length; i++) {
            // Cast both to bytes for comparison
            if (QueryId.unwrap(queryIds[i]) == QueryId.unwrap(queryId)) {
                if (i != length - 1) {
                    queryIds[i] = queryIds[length - 1];
                }
                queryIds.pop();
                return;
            }
        }
    }
}

/// @title QueryVerifierContract interface
/// @notice This interface defines the functions and events for interacting with the QueryVerifierContract.
interface QueryVerifierContract {
    function verify(
        bytes calldata proof,
        Query calldata query
    ) external returns (uint64);
}

event QuerySubmitted(QueryId indexed queryId, uint256 estimatedCost, uint256 escrowedAmount, Query query);
event QueryProofVerified(QueryId indexed queryId, bytes proof);
event EscrowedPaymentReclaimed(QueryId indexed queryId, uint256 escrowedAmount);
