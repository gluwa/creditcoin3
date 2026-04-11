// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {
    Ownable2Step,
    Ownable
} from "@openzeppelin/contracts/access/Ownable2Step.sol";

import {IVoteValidator} from "./abstract/IVoteValidator.sol";
import {VoteValidatorECDSAErrors} from "./error/VoteValidatorECDSAErrors.sol";

/// @notice ECDSA vote validator with threshold configuration and attester registry
contract VoteValidatorECDSA is IVoteValidator, Ownable2Step {
    uint256 public minAttesterCount;
    uint256 public thresholdNumerator;
    uint256 public thresholdDenominator;
    uint256 public thresholdAddition;

    mapping(address => bool) public isAttester;
    address[] public attesters;

    event AttesterSetUpdated(address[] newAttesters);
    event ThresholdUpdated(uint256 numerator, uint256 denominator, uint256 addition);
    event MinAttesterCountUpdated(uint256 newMin);

    constructor(
        address initialOwner,
        address[] memory initialAttesters,
        uint256 minAttesterCount_,
        uint256 thresholdNumerator_,
        uint256 thresholdDenominator_,
        uint256 thresholdAddition_
    ) Ownable(initialOwner) {
        if (initialOwner == address(0)) {
            revert VoteValidatorECDSAErrors.ZeroAddress();
        }
        if (initialAttesters.length < minAttesterCount_) {
            revert VoteValidatorECDSAErrors.BelowMinimumNumberOfAttesters();
        }
        if (thresholdDenominator_ == 0) {
            revert VoteValidatorECDSAErrors.InvalidThreshold();
        }

        minAttesterCount = minAttesterCount_;
        thresholdNumerator = thresholdNumerator_;
        thresholdDenominator = thresholdDenominator_;
        thresholdAddition = thresholdAddition_;

        for (uint256 i = 0; i < initialAttesters.length; i++) {
            address attester = initialAttesters[i];
            if (attester == address(0)) {
                revert VoteValidatorECDSAErrors.ZeroAddress();
            }
            if (isAttester[attester]) {
                revert VoteValidatorECDSAErrors.DuplicateAttester();
            }
            isAttester[attester] = true;
            attesters.push(attester);
        }
    }

    function validateVotes(
        bytes32 messageHash,
        bytes calldata votes
    ) external view override returns (bool) {
        bytes[] memory signatures = abi.decode(votes, (bytes[]));

        address[] memory signers = new address[](signatures.length);
        uint256 uniqueSigners = 0;

        for (uint256 i = 0; i < signatures.length; i++) {
            if (signatures[i].length != 65) {
                return false;
            }

            bytes32 r;
            bytes32 s;
            uint8 v;
            bytes memory sig = signatures[i];
            assembly {
                r := mload(add(sig, 32))
                s := mload(add(sig, 64))
                v := byte(0, mload(add(sig, 96)))
            }

            address signer = ecrecover(messageHash, v, r, s);
            if (!isAttester[signer]) {
                return false;
            }

            bool isDuplicate = false;
            for (uint256 j = 0; j < uniqueSigners; j++) {
                if (signers[j] == signer) {
                    isDuplicate = true;
                    break;
                }
            }
            if (isDuplicate) {
                return false;
            }

            signers[uniqueSigners] = signer;
            uniqueSigners++;
        }

        uint256 requiredVotes = calculateRequiredVotes(attesters.length);
        if (uniqueSigners < requiredVotes) {
            return false;
        }

        return true;
    }

    function calculateRequiredVotes(
        uint256 totalAttesters
    ) public view returns (uint256) {
        uint256 thresholdVotes = (totalAttesters * thresholdNumerator / thresholdDenominator)
            + thresholdAddition;
        return thresholdVotes > minAttesterCount ? thresholdVotes : minAttesterCount;
    }

    function addAttester(address attester) external onlyOwner {
        if (attester == address(0)) {
            revert VoteValidatorECDSAErrors.ZeroAddress();
        }
        if (isAttester[attester]) {
            revert VoteValidatorECDSAErrors.DuplicateAttester();
        }
        isAttester[attester] = true;
        attesters.push(attester);
        emit AttesterSetUpdated(attesters);
    }

    function removeAttester(address attester) external onlyOwner {
        if (!isAttester[attester]) {
            revert VoteValidatorECDSAErrors.InvalidAttester();
        }
        if (attesters.length - 1 < minAttesterCount) {
            revert VoteValidatorECDSAErrors.BelowMinimumNumberOfAttesters();
        }

        isAttester[attester] = false;
        for (uint256 i = 0; i < attesters.length; i++) {
            if (attesters[i] == attester) {
                attesters[i] = attesters[attesters.length - 1];
                attesters.pop();
                break;
            }
        }

        emit AttesterSetUpdated(attesters);
    }

    function updateAttesterSet(address[] calldata newAttesters) external onlyOwner {
        if (newAttesters.length < minAttesterCount) {
            revert VoteValidatorECDSAErrors.BelowMinimumNumberOfAttesters();
        }

        for (uint256 i = 0; i < attesters.length; i++) {
            isAttester[attesters[i]] = false;
        }
        delete attesters;

        for (uint256 i = 0; i < newAttesters.length; i++) {
            address attester = newAttesters[i];
            if (attester == address(0)) {
                revert VoteValidatorECDSAErrors.ZeroAddress();
            }
            if (isAttester[attester]) {
                revert VoteValidatorECDSAErrors.DuplicateAttester();
            }
            isAttester[attester] = true;
            attesters.push(attester);
        }

        emit AttesterSetUpdated(newAttesters);
    }

    function updateThreshold(
        uint256 numerator,
        uint256 denominator,
        uint256 addition
    ) external onlyOwner {
        if (denominator == 0) {
            revert VoteValidatorECDSAErrors.InvalidThreshold();
        }
        thresholdNumerator = numerator;
        thresholdDenominator = denominator;
        thresholdAddition = addition;
        emit ThresholdUpdated(numerator, denominator, addition);
    }

    function updateMinAttesterCount(uint256 newMin) external onlyOwner {
        if (attesters.length < newMin) {
            revert VoteValidatorECDSAErrors.CurrentSetBelowMinimum();
        }
        minAttesterCount = newMin;
        emit MinAttesterCountUpdated(newMin);
    }

    function validatorType() external pure override returns (string memory) {
        return "ecdsa";
    }
}
