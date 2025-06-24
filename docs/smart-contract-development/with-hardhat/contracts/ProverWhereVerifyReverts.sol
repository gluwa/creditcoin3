// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.0;

import "hardhat/console.sol";
import "./ProverForTesting.sol";

contract ProverWhereVerifyReverts is ProverForTesting {
    constructor(
        address _proceedsAccount,
        uint256 _costPerByte,
        uint256 _baseFee,
        uint64 _chainKey,
        string memory _displayName,
        uint64 _timeoutBlocks
    ) ProverForTesting(_proceedsAccount, _costPerByte, _baseFee, _chainKey, _displayName, _timeoutBlocks) {}

    function _call_verifier_verify(QueryId, bytes calldata) external pure override returns (VerifierResult memory) {
        revert("Reverted on purpose");
    }
}
