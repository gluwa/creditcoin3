pragma solidity ^0.8.0;

import './Prover.sol';

contract ProverForTesting is CreditcoinPublicProver {
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
