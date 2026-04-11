// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {IUSCQuoteValidator} from "../abstract/IUSCQuoteValidator.sol";

contract MockUSCQuoteValidator is IUSCQuoteValidator {
    bool public isValid = true;

    event QuoteValidated(
        bytes32 indexed chainKey,
        address indexed sender,
        bytes32 indexed messageHash,
        bytes quote,
        uint256 value
    );

    function setValid(bool nextValue) external {
        isValid = nextValue;
    }

    function validateQuote(
        bytes32 chainKey,
        address sender,
        bytes32 messageHash,
        bytes calldata quote
    ) external payable returns (bool) {
        emit QuoteValidated(chainKey, sender, messageHash, quote, msg.value);
        return isValid;
    }
}
