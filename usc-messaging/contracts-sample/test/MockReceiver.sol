// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {IMessageReceiver} from "../abstract/IMessageReceiver.sol";
import {IVoteValidator} from "../abstract/IVoteValidator.sol";

contract MockReceiver is IMessageReceiver {
    bytes32 public lastMessageId;
    uint256 public lastSourceChainId;
    address public lastEmitter;
    bytes public lastPayload;

    bool public shouldRevertReceive;
    bool public shouldRevertVoteValidator;
    IVoteValidator public customValidator;

    function setShouldRevertReceive(bool value) external {
        shouldRevertReceive = value;
    }

    function setShouldRevertVoteValidator(bool value) external {
        shouldRevertVoteValidator = value;
    }

    function setCustomValidator(IVoteValidator validator) external {
        customValidator = validator;
    }

    function receiveMessage(
        bytes32 messageId,
        uint256 sourceChainId,
        address emitterAddress,
        bytes calldata payload
    ) external override {
        if (shouldRevertReceive) {
            revert("MockRecipient: receive revert");
        }

        lastMessageId = messageId;
        lastSourceChainId = sourceChainId;
        lastEmitter = emitterAddress;
        lastPayload = payload;
    }

    function setTrustedInbox(address inbox, bool trusted) external override {
        // no-op
    }

    function isTrustedInbox(address inbox) external view override returns (bool) {
        return true;
    }
}
