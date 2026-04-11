// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {IUSCProofVerifier} from "../abstract/IUSCProofVerifier.sol";
import {BlockProverTypes} from "../common/BlockProverTypes.sol";

contract MockUSCProofVerifier is IUSCProofVerifier {
    bool public isValid = true;

    function setValid(bool nextValue) external {
        isValid = nextValue;
    }

    function verifyProofs(
        bytes32,
        uint64,
        bytes calldata,
        BlockProverTypes.InclusionProof calldata,
        BlockProverTypes.ContinuityProof calldata
    ) external view returns (bool) {
        return isValid;
    }
}
