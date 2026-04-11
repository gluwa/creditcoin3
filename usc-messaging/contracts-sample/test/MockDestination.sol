// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract MockDestination {
    bytes32 public lastIntentId;
    address public lastCaller;
    bool public shouldRevert;

    event DestinationCalled(bytes32 indexed intentId, address indexed caller);

    function setShouldRevert(bool value) external {
        shouldRevert = value;
    }

    function record(bytes32 intentId) external {
        if (shouldRevert) {
            revert("MockDestination: revert");
        }
        lastIntentId = intentId;
        lastCaller = msg.sender;
        emit DestinationCalled(intentId, msg.sender);
    }
}
