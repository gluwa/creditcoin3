// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @notice Dummy relayer contract for PoC. Accepts quotes and forwards payment to payee.
/// @dev In production, validates quote signature against whitelist.
contract DummyRelayerContract {
    address public payee;

    event FeeCollected(address indexed from, uint256 amount);

    constructor(address _payee) {
        require(_payee != address(0), "Invalid payee");
        payee = _payee;
    }

    /// @notice Validates quote and collects fee. Dummy: accepts any call, forwards msg.value to payee.
    function validateAndCollectFee(bytes calldata /* signedQuote */) external payable {
        if (msg.value > 0) {
            (bool ok,) = payee.call{value: msg.value}("");
            require(ok, "Transfer failed");
            emit FeeCollected(msg.sender, msg.value);
        }
    }
}
