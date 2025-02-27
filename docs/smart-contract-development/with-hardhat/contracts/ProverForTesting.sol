pragma solidity ^0.8.0;

import 'hardhat/console.sol';
import './Prover.sol';

contract ProverForTesting is CreditcoinPublicProver {
    uint64 private fakeVerifierResult;

    function mock_setVerifierResult(uint64 value) public {
        fakeVerifierResult = value;
    }

    // this will be called by submitQueryProof()
    function _call_verifier_verify(QueryId, bytes calldata) internal override view returns (uint64) {
        return fakeVerifierResult;
    }

    constructor(
        address _proceedsAccount, uint256 _costPerByte, uint256 _baseFee, uint64 _chainKey
    ) CreditcoinPublicProver(_proceedsAccount, _costPerByte, _baseFee, _chainKey) {}

    function getTotalEscrowBalance() public view returns (Balance) {
        return totalEscrowBalance;
    }

    function setQueryState(QueryId queryId) public onlyOwner {
        queries[queryId].state = QueryState.TimedOut;
    }
}
