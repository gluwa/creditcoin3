pragma solidity ^0.8.0;

import 'hardhat/console.sol';
import './ProverForTesting.sol';

contract ProverWhereVerifierGetResultSegmentsFails is ProverForTesting {
    function _call_verifier_get_result_segments(QueryId) internal override pure returns (ResultSegment[] memory) {
        revert('Failed on purpose');
    }

    constructor(
        address _proceedsAccount,
        uint256 _costPerByte,
        uint256 _baseFee,
        uint64 _chainKey,
        string memory _displayName
    ) ProverForTesting(_proceedsAccount, _costPerByte, _baseFee, _chainKey, _displayName) {}
}
