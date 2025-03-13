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
    uint256 public costPerByte;
    uint256 public baseFee;
    uint64 chainKey;
    string public displayName;

    constructor(
        address _proceedsAccount,
        uint256 _costPerByte,
        uint256 _baseFee,
        uint64 _chainKey,
        string memory _displayName
    ) Ownable() {
        verifier = QueryVerifierContract(PROOF_VERIFIER_ADDRESS);
        proceedsAccount = _proceedsAccount;
        totalEscrowBalance = Balance.wrap(0);
        costPerByte = _costPerByte;
        baseFee = _baseFee;
        chainKey = _chainKey;
        displayName = _displayName;

        emit ProverDeployed(address(this), msg.sender, _proceedsAccount, _costPerByte, _baseFee, _chainKey, _displayName);
    }

    function computeQueryCost(ChainQuery calldata query) public view returns (uint256) {
        // Cost function is based on the size of the query layoutsegments
        // I think it should also somehow include the distance between the required
        // block height and its nearest checkpoint or something of sorts (if distance
        // to a checkpoint determines the time prover needs to generate the proof)
        // not sure yet how to implement something like that

        // Calculate the total size of the query based on its layout segments
        uint256 totalBytes = 0;
        for (uint256 i = 0; i < query.layoutSegments.length; i++) {
            totalBytes += query.layoutSegments[i].size;
        }

        // Calculate the total cost as a function of the size and base cost
        uint256 cost = (totalBytes * costPerByte) + baseFee;

        return cost;
    }

    function updateCostPerByte(uint256 _newCostPerByte) external onlyOwner {
        costPerByte = _newCostPerByte;
        emit CostPerByteUpdated(_newCostPerByte);
    }

    function updateBaseFee(uint256 _newBaseFee) external onlyOwner {
        baseFee = _newBaseFee;
        emit BaseFeeUpdated(_newBaseFee);
    }

    function submitQuery(ChainQuery calldata query, address principal) public payable {
        require (query.chainId == chainKey, "Chain not supported");
        // Guards
        // QueryId may be computed differently if you'd like.
        QueryId queryId = QueryId.wrap(keccak256(abi.encode(query)));
        // require(queries[queryId].principal == address(0));
        // Need a more complex guard for the queries that allows replay attack protection.

        uint256 estimatedCost = computeQueryCost(query);
        require(msg.value >= estimatedCost, "Insufficient funds: msg.value must be >= estimatedCost");

        totalEscrowBalance = Balance.wrap(Balance.unwrap(totalEscrowBalance) + msg.value);

        if (queries[queryId].state != QueryState.Uninitialized) {
            revert("Query already exists");
        }

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
        require(queries[queryId].principal == msg.sender, 'Sender different from query.principal');

        QueryState state = queries[queryId].state;
        require(state == QueryState.TimedOut || state == QueryState.InvalidQuery, 'Query state does not allow reclaim');

        uint256 escrowedAmount = Balance.unwrap(queries[queryId].escrowedAmount);

        totalEscrowBalance = Balance.wrap(Balance.unwrap(totalEscrowBalance) - escrowedAmount);

        payable(msg.sender).transfer(escrowedAmount);

        queries[queryId].escrowedAmount = Balance.wrap(0);

        emit EscrowedPaymentReclaimed(queryId, escrowedAmount);
    }

    // wrapper which can be used to mock the verifier precompile for testing
    function _call_verifier_verify(QueryId queryId, bytes calldata proof) virtual internal returns (uint64) {
        return verifier.verify(proof, queries[queryId].query);
    }

    // submitQueryProof is called by the prover when a query's proof is ready.
    function submitQueryProof(QueryId queryId, bytes calldata proof) public onlyOwner returns (uint64) {
        // Fist verify the proof
        uint64 result = _call_verifier_verify(queryId, proof);

        // Calculate the prover's fee
        // Transfer the prover's fee to the prover
        uint256 proverFee = Balance.unwrap(queries[queryId].escrowedAmount);

        totalEscrowBalance = Balance.wrap(Balance.unwrap(totalEscrowBalance) - proverFee);

        // Send to proceedsAccount
        payable(proceedsAccount).transfer(proverFee);

        queries[queryId].escrowedAmount = Balance.wrap(0);

        // Check the result of the proof verification
        if (result == 0) {
            queries[queryId].state = QueryState.ResultAvailable;
        } else if (result == 1) {
            queries[queryId].state = QueryState.InvalidQuery;
        } else if (result == 2) {
            queries[queryId].state = QueryState.TimedOut;
        } else if (result == 3) {
            queries[queryId].state = QueryState.InvalidQuery;
        } else {
            queries[queryId].state = QueryState.InvalidQuery;
        }

        // Emit event with query ID, proof, and state
        emit QueryProofVerified(queryId, proof, queries[queryId].state);

        return result;
    }

    function withdrawProceeds() public onlyOwner {
        // allows the prover to withdraw the balance of the contract that's not
        // still escrowed

        // Compute the withdrawable balance
        uint256 contractBalance = address(this).balance;
        uint256 totalEscrowed = Balance.unwrap(totalEscrowBalance);
        uint256 withdrawable = contractBalance > totalEscrowed ? contractBalance - totalEscrowed : 0;

        require(withdrawable > 0, "No withdrawable proceeds available");

        // Transfer the amount to the proceeds account
        payable(proceedsAccount).transfer(withdrawable);

        emit ProceedsWithdrawn(proceedsAccount, withdrawable);
    }

    function getUnprocessedQueries() public view returns (ChainQuery[] memory) {
        uint256 unprocessedCount;

        for (uint256 i = 0; i < queryIds.length; i++) {
            if (queries[queryIds[i]].state == QueryState.Submitted) {
                unprocessedCount++;
            }
        }

        ChainQuery[] memory unprocessedQueries = new ChainQuery[](unprocessedCount);
        uint256 index;

        for (uint256 i = 0; i < queryIds.length; i++) {
            if (queries[queryIds[i]].state == QueryState.Submitted) {
                unprocessedQueries[index] = queries[queryIds[i]].query;
                index++;
            }
        }

        return unprocessedQueries;
    }

    function removeQueryId(QueryId queryId) public onlyOwner {
        uint256 length = queryIds.length;
        for (uint256 i = 0; i < length; i++) {
            // Cast both to bytes for comparison
            if (QueryId.unwrap(queryIds[i]) == QueryId.unwrap(queryId)) {
                if (i != length - 1) {
                    queryIds[i] = queryIds[length - 1];
                }
                queryIds.pop();
                delete queries[queryId];
                return;
            }
        }
    }

    function getQueryResultSegments(QueryId queryId) public returns (ResultSegment[] memory) {
        QueryState state = queries[queryId].state;
        require(state == QueryState.ResultAvailable, "Query result not available");

        ResultSegment[] memory resultSegments = verifier.get_result_segments(queryId);

        return resultSegments;
    }
}

/// @title QueryVerifierContract interface
/// @notice This interface defines the functions and events for interacting with the QueryVerifierContract.
interface QueryVerifierContract {
    function verify(
        bytes calldata proof,
        ChainQuery calldata query
    ) external returns (uint64);

    function get_result_segments(
        QueryId queryId
    ) external returns (ResultSegment[] memory);
}

event ProverDeployed(address indexed contractAddress, address indexed owner, address proceedsAccount, uint256 costPerByte, uint256 baseFee, uint64 chainKey, string displayName);
event QuerySubmitted(QueryId indexed queryId, uint256 estimatedCost, uint256 escrowedAmount, ChainQuery chainQuery);
event QueryProofVerified(QueryId indexed queryId, bytes proof, QueryState state);
event EscrowedPaymentReclaimed(QueryId indexed queryId, uint256 escrowedAmount);
event ProceedsWithdrawn(address indexed proceedsAccount, uint256 amount);
event CostPerByteUpdated(uint256 newCostPerByte);
event BaseFeeUpdated(uint256 newBaseFee);
