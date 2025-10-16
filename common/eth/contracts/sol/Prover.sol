// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.24;

import {
    Balance,
    ChainQuery,
    QueryId,
    QueryDetails,
    QueryState,
    ResultSegment,
    VerifierResult
} from "./Types.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
address constant PROOF_VERIFIER_ADDRESS = 0x0000000000000000000000000000000000000Be9;

/// @title Gluwa Public Prover interface
/// @author Gluwa
/// @notice Defines how to query for events on 3rd party chains
interface ICreditcoinPublicProver {
    /// @notice Read information about a query from contract storage
    /// @param queryId The query ID in question
    /// @return queryDetails A QueryDetails struct for the given ID
    function getQueryDetails(bytes32 queryId) external view returns (QueryDetails memory queryDetails);
}

/// @title Gluwa Public Prover contract
/// @author Gluwa
/// @notice Canonical implementation of the ICreditcoinPublicProver interface
contract CreditcoinPublicProver is ICreditcoinPublicProver, Ownable {
    /// @notice A mapping queries processed by this contract
    mapping(QueryId => QueryDetails) public queries;
    /// @notice An array of query IDs processed by this contract
    QueryId[] public queryIds;
    Balance internal totalEscrowBalance;
    IQueryVerifierContract private verifier;
    address private proceedsAccount;
    /// @notice The cost per byte configured on this contract
    uint256 public costPerByte;
    /// @notice The base fee configured on this contract
    uint256 public baseFee;
    uint64 private chainKey;
    /// @notice A human readable way to identify this contract
    string public displayName;
    uint64 private timeout = 100;

    /// @notice Create a new prover contract
    /// @param _proceedsAccount The address to which proceeds will be paid out
    /// @param _costPerByte The cost per byte configured on this contract
    /// @param _baseFee The base fee configured on this contract
    /// @param _chainKey Chain key which this contract is valid for
    /// @param _displayName A human readable way to identify this contract
    /// @param _timeout Number of seconds before a query is considered to be timed out
    constructor(
        address _proceedsAccount,
        uint256 _costPerByte,
        uint256 _baseFee,
        uint64 _chainKey,
        string memory _displayName,
        uint64 _timeout
    ) Ownable(msg.sender) {
        verifier = IQueryVerifierContract(PROOF_VERIFIER_ADDRESS);
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

    /// @notice Calculate how much processing a query will cost
    /// @param query The query in question
    /// @return A numerical representation of cost
    function computeQueryCost(ChainQuery calldata query) public view returns (uint256) {
        // Cost function is based on the size of the query layoutsegments
        // I think it should also somehow include the distance between the required
        // block height and its nearest checkpoint or something of sorts (if distance
        // to a checkpoint determines the time prover needs to generate the proof)
        // not sure yet how to implement something like that

        // Calculate the total size of the query based on its layout segments
        uint256 totalBytes = 0;
        for (uint256 i = 0; i < query.layoutSegments.length; ++i) {
            totalBytes += query.layoutSegments[i].size;
        }

        // Calculate the total cost as a function of the size and base cost
        uint256 cost = (totalBytes * costPerByte) + baseFee;

        return cost;
    }

    /// @notice Update cost per byte configured on this contract
    /// @param _newCostPerByte The new cost per byte to be applied to future queries
    function updateCostPerByte(uint256 _newCostPerByte) external onlyOwner {
        costPerByte = _newCostPerByte;
        emit CostPerByteUpdated(_newCostPerByte);
    }

    /// @notice Update base fee configured on this contract
    /// @param _newBaseFee The new fee to be applied to future queries
    function updateBaseFee(uint256 _newBaseFee) external onlyOwner {
        baseFee = _newBaseFee;
        emit BaseFeeUpdated(_newBaseFee);
    }

    /// @notice Calculate a query ID for further use internally
    /// @param query The query in question
    /// @return A query ID which can be used in other functions
    function computeQueryId(ChainQuery calldata query) internal pure returns (QueryId) {
        return QueryId.wrap(keccak256(abi.encode(query)));
    }

    /// @notice Submit a query to this smart contract. This is the main entry-point for smart contract developers
    /// @param query The query in question
    /// @param principal Address of submitter associated with this query
    function submitQuery(ChainQuery calldata query, address principal) public payable {
        require(query.chainId == chainKey, "Chain not supported");
        QueryId queryId = computeQueryId(query);
        // require(queries[queryId].principal == address(0));
        // Need a more complex guard for the queries that allows replay attack protection.

        uint256 estimatedCost = computeQueryCost(query);
        // solhint-disable-next-line gas-strict-inequalities
        require(msg.value >= estimatedCost, "msg.value < estimatedCost");

        totalEscrowBalance = Balance.wrap(Balance.unwrap(totalEscrowBalance) + msg.value);

        // Prevent resubmission of invalid queries
        if (queries[queryId].state == QueryState.InvalidQuery) {
            revert("Cannot resubmit an invalid query");
        }

        // Prevent resubmission if result already available
        if (queries[queryId].state == QueryState.ResultAvailable) {
            revert("Query result available");
        }

        // Prevent resubmission of already submitted queries unless timed out
        if (queries[queryId].state == QueryState.Submitted && !isQueryTimedOut(queryId)) {
            revert("Query submitted and pending");
        }

        // We implicitly allow resubmission of queries in the state QueryProcessingFailed

        // Store query details
        queries[queryId].state = QueryState.Submitted;
        queries[queryId].query.chainId = query.chainId;
        queries[queryId].query.height = query.height;
        queries[queryId].query.index = query.index;
        delete queries[queryId].query.layoutSegments; // clear existing storage array

        for (uint256 i = 0; i < query.layoutSegments.length; ++i) {
            queries[queryId].query.layoutSegments.push(query.layoutSegments[i]);
        }
        // .result doesn't need to be set here
        queries[queryId].escrowedAmount = Balance.wrap(msg.value);
        queries[queryId].principal = principal;
        queries[queryId].estimatedCost = Balance.wrap(estimatedCost);
        queries[queryId].timestamp = block.timestamp;

        // Add to unprocessed queries
        queryIds.push(queryId);

        emit QuerySubmitted(queryId, estimatedCost, msg.value, query);
    }

    /// @notice Read query result from contract storage
    /// @param query The query in question
    /// @return memory An array of ResultSegment elements
    function getQueryResult(ChainQuery calldata query) public view returns (ResultSegment[] memory) {
        require(query.chainId == chainKey, "Chain not supported");
        QueryId queryId = computeQueryId(query);
        if (queries[queryId].state == QueryState.ResultAvailable) {
            return queries[queryId].resultSegments;
        }
        return new ResultSegment[](0);
    }

    /// @notice Initiate return of an escrowed payment back to sender
    /// @param queryId The query ID in question
    function reclaimEscrowedPayment(QueryId queryId) public {
        require(queries[queryId].principal == msg.sender, "sender != query.principal");

        QueryState state = queries[queryId].state;
        // Explicitly revert if the state is ResultAvailable
        require(state != QueryState.ResultAvailable, "Query result available");

        // Allow reclaim if timeout has passed OR if the query processing failed
        bool queryProcessingFailed = (state == QueryState.InvalidQuery || state == QueryState.QueryProcessingFailed);

        require(
            queryProcessingFailed || isQueryTimedOut(queryId), "Query not invalid nor timed out"
        );

        // Reclaim the escrowed amount
        helperReclaimEscrowPayment(queryId);
    }

    /// @notice wrapper which can be used to mock the verifier precompile for testing
    /// @param queryId The query ID in question
    /// @param proof The proof bytes which need to be verified
    /// @return memory A VerifierResult struct
    function _callVerifierVerify(QueryId queryId, bytes calldata proof)
        external
        virtual
        returns (VerifierResult memory)
    {
        return verifier.verify(proof, queries[queryId].query);
    }

    /// @notice Called by the prover when a query's proof is ready
    /// @param queryId The query ID in question
    /// @param proof The proof bytes which need to be verified
    /// @return memory An array of ResultSegment elements
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

        queries[queryId].escrowedAmount = Balance.wrap(0);

        queries[queryId].state = QueryState.ResultAvailable;
        setQueryResultSegments(queryId, verifierResult.resultSegments);

        // Send to proceedsAccount
        payable(proceedsAccount).transfer(proverFee);

        // Emit event with query ID, proof, and state
        emit QueryProofVerified(queryId, verifierResult.resultSegments, queries[queryId].state);

        return verifierResult.resultSegments;
    }

    /// @notice Withdraw all proceeds to `proceedsAccount`
    function withdrawProceeds() public onlyOwner {
        // allows the prover to withdraw the balance of the contract that's not
        // still escrowed

        // Compute the withdrawable balance
        uint256 contractBalance = address(this).balance;
        uint256 totalEscrowed = Balance.unwrap(totalEscrowBalance);
        uint256 withdrawable = contractBalance > totalEscrowed ? contractBalance - totalEscrowed : 0;

        require(withdrawable > 0, "No withdrawable funds available");

        // Transfer the amount to the proceeds account
        payable(proceedsAccount).transfer(withdrawable);

        emit ProceedsWithdrawn(proceedsAccount, withdrawable);
    }

    /// @notice Helper function to extract all queries which have not been processed yet
    /// @return memory An array of ChainQuery elements
    function getUnprocessedQueries() public view returns (ChainQuery[] memory) {
        ChainQuery[] memory temp = new ChainQuery[](queryIds.length);
        uint256 count = 0;

        for (uint256 i = 0; i < queryIds.length; ++i) {
            QueryDetails storage current = queries[queryIds[i]];
            if (current.state == QueryState.Submitted && !isQueryTimedOut(queryIds[i])) {
                // solhint-disable-next-line gas-increment-by-one
                temp[count++] = current.query;
            }
        }

        ChainQuery[] memory result = new ChainQuery[](count);
        for (uint256 i = 0; i < count; ++i) {
            result[i] = temp[i];
        }

        return result;
    }

    /// @notice Helper function to mark a query as invalid
    /// @param queryId The query ID in question
    /// @param reason Human readable reason
    function markAsInvalid(QueryId queryId, string memory reason) public onlyOwner {
        require(queries[queryId].state != QueryState.Uninitialized, "Query not found");
        require(queries[queryId].state != QueryState.ResultAvailable, "Query result available");

        queries[queryId].state = QueryState.InvalidQuery;

        // Reclaim escrowed payment
        helperReclaimEscrowPayment(queryId);

        emit QueryMarkedInvalid(queryId, reason);
    }

    /// @notice Helper function to mark a query as failed
    /// @param queryId The query ID in question
    /// @param reason Human readable reason
    function markProcessingFailed(QueryId queryId, string memory reason) public onlyOwner {
        require(queries[queryId].state != QueryState.Uninitialized, "Query not found");
        require(queries[queryId].state != QueryState.ResultAvailable, "Query result available");

        queries[queryId].state = QueryState.QueryProcessingFailed;

        // Reclaim escrowed payment
        helperReclaimEscrowPayment(queryId);

        emit QueryProcessingFailed(queryId, reason);
    }

    /// @notice Helper function to return escrowed amount for a query
    /// @param queryId The query ID in question
    function helperReclaimEscrowPayment(QueryId queryId) private {
        // Repay the escrowed amount to the principal
        uint256 escrowedAmount = Balance.unwrap(queries[queryId].escrowedAmount);
        totalEscrowBalance = Balance.wrap(Balance.unwrap(totalEscrowBalance) - escrowedAmount);
        queries[queryId].escrowedAmount = Balance.wrap(0);

        payable(queries[queryId].principal).transfer(escrowedAmount);

        emit EscrowedPaymentReclaimed(queryId, escrowedAmount);
    }

    /// @notice Check if a query was submitted more than `timeout` seconds ago
    /// @param queryId The query ID in question
    /// @return A bool if the query had already timed out
    function isQueryTimedOut(QueryId queryId) public view returns (bool) {
        return block.timestamp > queries[queryId].timestamp + timeout;
    }

    /// @notice Read information about a query from contract storage
    /// @param queryId The query ID in question
    /// @return queryDetails A QueryDetails struct for the given ID
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

    /// @notice Copy result segments from memory to internal contract storage
    /// @param queryId The query ID for which we want to store these result segments
    /// @param resultSegments The result segments we want to copy
    function setQueryResultSegments(QueryId queryId, ResultSegment[] memory resultSegments) private {
        delete queries[queryId].resultSegments; // clear existing storage array

        for (uint256 i = 0; i < resultSegments.length; ++i) {
            queries[queryId].resultSegments.push(resultSegments[i]); // push each element
        }
    }
}

/// @title IQueryVerifierContract interface
/// @author Gluwa
/// @notice This interface defines the functions and events for interacting with the IQueryVerifierContract.
interface IQueryVerifierContract {
    /// @notice Verifies the query proof
    /// @param proof Proof bytes
    /// @param query The query itself
    /// @return memory The result of the verify operation
    function verify(bytes calldata proof, ChainQuery calldata query) external returns (VerifierResult memory);
}

/// @notice Emitted when this smart contract is deployed
/// @param contractAddress The EVM address at which this contract was deployed
/// @param owner The account who deployed this contract
/// @param proceedsAccount The address to which proceeds will be paid out
/// @param costPerByte The cost per byte configured on this contract
/// @param baseFee The base fee configured on this contract
/// @param chainKey Chain key which this contract is valid for
/// @param displayName A human readable way to identify this contract
/// @param timeout Number of seconds before a query is considered to be timed out
event ProverDeployed(
    address indexed contractAddress,
    address indexed owner,
    address proceedsAccount,
    uint256 indexed costPerByte,
    uint256 baseFee,
    uint64 chainKey,
    string displayName,
    uint64 timeout
);

/// @notice Emitted when a query is submitted to this smart contract
/// @param queryId The ID of the query in question
/// @param estimatedCost The computed estimated cost of this query
/// @param escrowedAmount The amount sender will actually pay when results become available
/// @param chainQuery The query itself
event QuerySubmitted(QueryId indexed queryId, uint256 estimatedCost, uint256 escrowedAmount, ChainQuery chainQuery);

/// @notice Emitted when results for a query become available
/// @param queryId The ID of the query in question
/// @param resultSegments The result segments for this query
/// @param state The state of this query. Should be QueryState.ResultAvailable
event QueryProofVerified(QueryId indexed queryId, ResultSegment[] resultSegments, QueryState state);

/// @notice Emitted when a query is marked as invalid
/// @param queryId The ID of the query in question
/// @param reason Human readable reason
event QueryMarkedInvalid(QueryId indexed queryId, string reason);

/// @notice Emitted when processing a query fails
/// @param queryId The ID of the query in question
/// @param reason Human readable reason
event QueryProcessingFailed(QueryId indexed queryId, string reason);

/// @notice Emitted when an escrowed payment has been returned back to sender
/// @param queryId The ID of the query in question
/// @param escrowedAmount The amount which was returned
event EscrowedPaymentReclaimed(QueryId indexed queryId, uint256 indexed escrowedAmount);

/// @notice Emitted when prover withdraws their proceeds
/// @param proceedsAccount The address which received the proceeds
/// @param amount The amount which was withdrawn
event ProceedsWithdrawn(address indexed proceedsAccount, uint256 indexed amount);

/// @notice Emitted when the cost per byte fee is updated
/// @param newCostPerByte The value of the new cost per byte
event CostPerByteUpdated(uint256 indexed newCostPerByte);

/// @notice Emitted when the contract base fee is updated
/// @param newBaseFee The value of the new base fee
event BaseFeeUpdated(uint256 indexed newBaseFee);
