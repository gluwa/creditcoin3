// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.0;

import 'hardhat/console.sol';
import './ProverForTesting.sol';

contract ProverWhereVerifierGetResultSegmentsReverts is ProverForTesting {
    function _call_verifier_get_result_segments(QueryId) internal override pure returns (ResultSegment[] memory) {
        revert('Reverted on purpose');
    }

    constructor(
        address _proceedsAccount,
        uint256 _costPerByte,
        uint256 _baseFee,
        uint64 _chainKey,
        string memory _displayName,
        uint64 _timeoutBlocks
    ) ProverForTesting(_proceedsAccount, _costPerByte, _baseFee, _chainKey, _displayName, _timeoutBlocks) {}
}
