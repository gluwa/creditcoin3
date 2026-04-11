// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

interface IUSCQuoteValidator {
    /// @notice Validates fee quote data for outbound bridge initiation.
    function validateQuote(
        bytes32 chainKey,
        address sender,
        bytes32 messageHash,
        bytes calldata quote
    ) external payable returns (bool);
}
