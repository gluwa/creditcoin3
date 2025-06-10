// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.0;

import "hardhat/console.sol";
import "./ProverForTesting.sol";

contract ProverWhereVerifierGetResultSegmentsReverts is ProverForTesting {
    // TODO: Maybe replace this with ProverWhereGetQueryDetailsReverts. But the precompile is no longer called under the hood,
    // so this checking may be unnecessary.

    constructor(
        address _proceedsAccount,
        uint256 _costPerByte,
        uint256 _baseFee,
        uint64 _chainKey,
        string memory _displayName,
        uint64 _timeoutBlocks
    ) ProverForTesting(_proceedsAccount, _costPerByte, _baseFee, _chainKey, _displayName, _timeoutBlocks) {}
}
