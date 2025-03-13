pragma solidity ^0.8.0;

import 'hardhat/console.sol';
import './Prover.sol';

contract ProverForTesting is CreditcoinPublicProver {
    uint64 private fakeVerifierResult;
    ResultSegment[] private fakeQueryResultSegments;

    function mock_setVerifierResult(uint64 value) public {
        fakeVerifierResult = value;
    }

    function mock_pushQueryResultSegment(ResultSegment memory value) public {
        fakeQueryResultSegments.push(value);
    }

    // this will be called by submitQueryProof()
    function _call_verifier_verify(QueryId, bytes calldata) internal override view returns (uint64) {
        return fakeVerifierResult;
    }

    // this will be called by getQueryResultSegments()
    function _call_verifier_get_result_segments(QueryId) internal override view returns (ResultSegment[] memory) {
        return fakeQueryResultSegments;
    }

    constructor(
        address _proceedsAccount,
        uint256 _costPerByte,
        uint256 _baseFee,
        uint64 _chainKey,
        string memory _displayName
    ) CreditcoinPublicProver(_proceedsAccount, _costPerByte, _baseFee, _chainKey, _displayName) {}

    function getTotalEscrowBalance() public view returns (Balance) {
        return totalEscrowBalance;
    }

    function allQueryIds() external view returns (QueryId[] memory) {
        return queryIds;
    }

    function mock_setQueryState(QueryId queryId, QueryState newState) public onlyOwner {
        queries[queryId].state = newState;
    }

    function mock_drainBalance(uint256 howMuch) public onlyOwner {
        payable(0).transfer(howMuch);
    }
}
