// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {IASCProofVerifier} from "@gluwa/usc-contracts/write-ability/abstract/IASCProofVerifier.sol";
import {BlockProverTypes} from "@gluwa/usc-contracts/write-ability/common/BlockProverTypes.sol";

/// @notice Test double for IASCProofVerifier: hands back a canned `encodedTransaction` instead of
///         verifying anything, so BridgeHub's decode/routing logic can be exercised without a real
///         block-prover precompile.
contract MockASCProofVerifier is IASCProofVerifier {
    bytes public nextEncodedTransaction;
    bool public shouldRevert;

    function setNextEncodedTransaction(bytes calldata data) external {
        nextEncodedTransaction = data;
    }

    function setShouldRevert(bool value) external {
        shouldRevert = value;
    }

    function verifyProofs(
        bytes32, /* chainKey */
        uint64, /* blockHeight */
        BlockProverTypes.InclusionProof calldata, /* inclusionProof */
        BlockProverTypes.ContinuityProof calldata /* continuityProof */
    ) external view returns (bytes memory) {
        require(!shouldRevert, "MockASCProofVerifier: forced revert");
        return nextEncodedTransaction;
    }

    function calculateTxIndex(
        BlockProverTypes.InclusionProof calldata /* inclusionProof */
    )
        external
        pure
        returns (uint64)
    {
        return 0;
    }
}
