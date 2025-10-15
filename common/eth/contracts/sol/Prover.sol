// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.24;

import "./Types.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
address constant PROOF_VERIFIER_ADDRESS = 0x0000000000000000000000000000000000000Be9;

interface ICreditcoinPublicProver {
    function getQueryDetails(bytes32 queryId) external view returns (QueryDetails memory queryDetails);
}

contract CreditcoinPublicProver is ICreditcoinPublicProver, Ownable {
    mapping(QueryId => QueryDetails) public queries;
    QueryId[] public queryIds;
    Balance totalEscrowBalance;
    QueryVerifierContract verifier;
    address proceedsAccount;
    uint256 public costPerByte;
    uint256 public baseFee;
    uint64 chainKey;
    string public displayName;
    uint64 timeout = 100;

    constructor(
        address _proceedsAccount,
        uint256 _costPerByte,
        uint256 _baseFee,
        uint64 _chainKey,
        string memory _displayName,
        uint64 _timeout
    ) Ownable(msg.sender) {
        verifier = QueryVerifierContract(PROOF_VERIFIER_ADDRESS);
        proceedsAccount = _proceedsAccount;
        totalEscrowBalance = Balance.wrap(0);
        costPerByte = _costPerByte;
        baseFee = _baseFee;
        chainKey = _chainKey;
        displayName = _displayName;
        timeout = _timeout;

        emit ProverDeployed(
            address(this), msg.sender, _proceedsAccount, _costPerByte, _baseFee, _chainKey, _displayName, _timeout
        );
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

    function computeQueryId(ChainQuery calldata query) internal pure returns (QueryId) {
        return QueryId.wrap(keccak256(abi.encode(query)));
    }

    function submitQuery(ChainQuery calldata query, address principal) public payable {
        require(query.chainId == chainKey, "Chain not supported");
        QueryId queryId = computeQueryId(query);
        // require(queries[queryId].principal == address(0));
        // Need a more complex guard for the queries that allows replay attack protection.

        uint256 estimatedCost = computeQueryCost(query);
        require(msg.value >= estimatedCost, "Insufficient funds: msg.value must be >= estimatedCost");

        totalEscrowBalance = Balance.wrap(Balance.unwrap(totalEscrowBalance) + msg.value);

        // Prevent resubmission of invalid queries
        if (queries[queryId].state == QueryState.InvalidQuery) {
            revert("Cannot resubmit an invalid query");
        }

        // Prevent resubmission if result already available
        if (queries[queryId].state == QueryState.ResultAvailable) {
            revert("Query proof already generated, check contract storage for results");
        }

        // Prevent resubmission of already submitted queries unless timed out
        if (queries[queryId].state == QueryState.Submitted && !isQueryTimedOut(queryId)) {
            revert("Query already submitted and still pending");
        }

        // We implicitly allow resubmission of queries in the state QueryProcessingFailed

        // Store query details
        // .state
        queries[queryId].state = QueryState.Submitted;
        // .query
        queries[queryId].query.chainId = query.chainId;
        queries[queryId].query.height = query.height;
        queries[queryId].query.index = query.index;
        delete queries[queryId].query.layoutSegments; // clear existing storage array

        for (uint256 i = 0; i < query.layoutSegments.length; i++) {
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
        queries[queryId].timestamp = block.timestamp;

        // Add to unprocessed queries
        queryIds.push(queryId);

        // Emit event
        emit QuerySubmitted(queryId, estimatedCost, msg.value, query);
    }

    function getQueryResult(ChainQuery calldata query) public view returns (ResultSegment[] memory) {
        require(query.chainId == chainKey, "Chain not supported");
        QueryId queryId = computeQueryId(query);
        if (queries[queryId].state == QueryState.ResultAvailable) {
            return queries[queryId].resultSegments;
        }
        return new ResultSegment[](0);
    }

    function reclaimEscrowedPayment(QueryId queryId) public {
        require(queries[queryId].principal == msg.sender, "Sender different from query.principal");

        QueryState state = queries[queryId].state;
        // Explicitly revert if the state is ResultAvailable
        require(state != QueryState.ResultAvailable, "Cannot reclaim: query result is available");

        // Allow reclaim if timeout has passed OR if the query processing failed
        bool queryProcessingFailed = (state == QueryState.InvalidQuery || state == QueryState.QueryProcessingFailed);

        require(
            queryProcessingFailed || isQueryTimedOut(queryId), "Cannot reclaim: neither timeout nor invalid query state met"
        );

        // Reclaim the escrowed amount
        helperReclaimEscrowPayment(queryId);
    }

    // wrapper which can be used to mock the verifier precompile for testing
    function _callVerifierVerify(QueryId queryId, bytes calldata proof)
        external
        virtual
        returns (VerifierResult memory)
    {
        return verifier.verify(proof, queries[queryId].query);
    }

    // submitQueryProof is called by the prover when a query's proof is ready.
    function submitQueryProof(QueryId queryId, bytes calldata proof)
        public
        onlyOwner
        returns (ResultSegment[] memory)
    {
        // Check if timeout has occurred
        if (isQueryTimedOut(queryId)) {
            revert("Query has timed out");
        }

        VerifierResult memory verifierResult = this._callVerifierVerify(queryId, proof);
        // note: since https://github.com/gluwa/creditcoin3-next/pull/608
        // non Success statuses are no longer returned, instead the precompile reverts

        // Calculate the prover's fee
        // Transfer the prover's fee to the prover
        uint256 proverFee = Balance.unwrap(queries[queryId].escrowedAmount);

        totalEscrowBalance = Balance.wrap(Balance.unwrap(totalEscrowBalance) - proverFee);

        // Send to proceedsAccount
        payable(proceedsAccount).transfer(proverFee);

        queries[queryId].escrowedAmount = Balance.wrap(0);

        queries[queryId].state = QueryState.ResultAvailable;
        setQueryResultSegments(queryId, verifierResult.resultSegments);

        // Emit event with query ID, proof, and state
        emit QueryProofVerified(queryId, verifierResult.resultSegments, queries[queryId].state);

        return verifierResult.resultSegments;
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
        ChainQuery[] memory temp = new ChainQuery[](queryIds.length);
        uint256 count = 0;

        for (uint256 i = 0; i < queryIds.length; i++) {
            QueryDetails storage current = queries[queryIds[i]];
            if (current.state == QueryState.Submitted && !isQueryTimedOut(queryIds[i])) {
                temp[count++] = current.query;
            }
        }

        ChainQuery[] memory result = new ChainQuery[](count);
        for (uint256 i = 0; i < count; i++) {
            result[i] = temp[i];
        }

        return result;
    }

    function markAsInvalid(QueryId queryId, string memory reason) public onlyOwner {
        require(queries[queryId].state != QueryState.Uninitialized, "Query not found");
        require(queries[queryId].state != QueryState.ResultAvailable, "Cannot mark as invalid: result available");

        queries[queryId].state = QueryState.InvalidQuery;

        // Reclaim escrowed payment
        helperReclaimEscrowPayment(queryId);

        emit QueryMarkedInvalid(queryId, reason);
    }

    function markProcessingFailed(QueryId queryId, string memory reason) public onlyOwner {
        require(queries[queryId].state != QueryState.Uninitialized, "Query not found");
        require(queries[queryId].state != QueryState.ResultAvailable, "Cannot mark processing as failed: result available");

        queries[queryId].state = QueryState.QueryProcessingFailed;

        // Reclaim escrowed payment
        helperReclaimEscrowPayment(queryId);

        emit QueryProcessingFailed(queryId, reason);
    }

    function helperReclaimEscrowPayment(QueryId queryId) private {
        // Repay the escrowed amount to the principal
        uint256 escrowedAmount = Balance.unwrap(queries[queryId].escrowedAmount);
        totalEscrowBalance = Balance.wrap(Balance.unwrap(totalEscrowBalance) - escrowedAmount);
        payable(queries[queryId].principal).transfer(escrowedAmount);
        queries[queryId].escrowedAmount = Balance.wrap(0);

        emit EscrowedPaymentReclaimed(queryId, escrowedAmount);
    }

    function isQueryTimedOut(QueryId queryId) public view returns (bool) {
        return block.timestamp > queries[queryId].timestamp + timeout;
    }

    function getQueryDetails(bytes32 queryId)
        external
        view
        virtual
        override
        returns (QueryDetails memory queryDetails)
    {
        queryDetails = queries[QueryId.wrap(queryId)];
        require(queryDetails.state != QueryState.Uninitialized, "No such query");
        return queryDetails;
    }

    // Necessary to satisfy compiler
    function setQueryResultSegments(QueryId queryId, ResultSegment[] memory resultSegments) private {
        delete queries[queryId].resultSegments; // clear existing storage array

        for (uint256 i = 0; i < resultSegments.length; i++) {
            queries[queryId].resultSegments.push(resultSegments[i]); // push each element
        }
    }
}

/// @title QueryVerifierContract interface
/// @notice This interface defines the functions and events for interacting with the QueryVerifierContract.
interface QueryVerifierContract {
    function verify(bytes calldata proof, ChainQuery calldata query) external returns (VerifierResult memory);
}

event ProverDeployed(
    address indexed contractAddress,
    address indexed owner,
    address proceedsAccount,
    uint256 costPerByte,
    uint256 baseFee,
    uint64 chainKey,
    string displayName,
    uint64 timeout
);

event QuerySubmitted(QueryId indexed queryId, uint256 estimatedCost, uint256 escrowedAmount, ChainQuery chainQuery);

event QueryProofVerified(QueryId indexed queryId, ResultSegment[] resultSegments, QueryState state);

event QueryMarkedInvalid(QueryId indexed queryId, string reason);

event QueryProcessingFailed(QueryId indexed queryId, string reason);

event EscrowedPaymentReclaimed(QueryId indexed queryId, uint256 escrowedAmount);

event ProceedsWithdrawn(address indexed proceedsAccount, uint256 amount);

event CostPerByteUpdated(uint256 newCostPerByte);

event BaseFeeUpdated(uint256 newBaseFee);
