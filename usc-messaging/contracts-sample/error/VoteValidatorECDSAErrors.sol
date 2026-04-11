// SPDX-License-Identifier: MIT
pragma solidity >0.8.0 <0.9.0;

library VoteValidatorECDSAErrors {
    error ZeroAddress();
    error InvalidAttester();
    error BelowMinimumNumberOfAttesters();
    error DuplicateAttester();
    error InvalidThreshold();
    error CurrentSetBelowMinimum();
}
