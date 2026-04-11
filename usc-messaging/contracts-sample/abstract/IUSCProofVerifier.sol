// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {BlockProverTypes} from "../common/BlockProverTypes.sol";

interface IUSCProofVerifier {
    /// @notice Verifies transaction inclusion and chain continuity for a source chain.
    function verifyProofs(
        bytes32 chainKey,
        uint64 blockHeight,
        bytes calldata encodedTransaction,
        BlockProverTypes.InclusionProof calldata inclusionProof,
        BlockProverTypes.ContinuityProof calldata continuityProof
    ) external view returns (bool);
}
