// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {
    Ownable2Step,
    Ownable
} from "@openzeppelin/contracts/access/Ownable2Step.sol";

import {IInbox} from "./abstract/IInbox.sol";
import {IMessageReceiver} from "./abstract/IMessageReceiver.sol";
import {IVoteValidator} from "./abstract/IVoteValidator.sol";
import {InboxErrors} from "./error/InboxErrors.sol";

contract Inbox is IInbox, Ownable2Step {
    bytes32 public immutable override localChainKey;
    uint256 public immutable override creditcoinChainId;
    
    IVoteValidator public override defaultVoteValidator;
    address public override bridgeReceiver;

    mapping(bytes32 => uint256) private _processedAt;
    mapping(bytes32 => bool) private _validatedMessages;
    mapping(bytes32 => bool) private _isPending;
    mapping(bytes32 => bool) private _validationFailed;

    struct PendingMessage {
        address emitterAddress;
        bytes payloadData;
    }

    mapping(bytes32 => PendingMessage) private _pendingMessages;

    constructor(
        bytes32 chainKey,
        uint256 creditcoinChainId_,
        IVoteValidator initialValidator,
        address bridgeReceiver_,
        address initialOwner
    ) Ownable(initialOwner) {
        if (chainKey == bytes32(0)) {
            revert InboxErrors.InvalidChainKey();
        }
        if (creditcoinChainId_ == 0) {
            revert InboxErrors.InvalidChainId();
        }
        if (address(initialValidator) == address(0)) {
            revert InboxErrors.ZeroAddress();
        }
        if (initialOwner == address(0)) {
            revert InboxErrors.ZeroAddress();
        }
        if (bridgeReceiver_ == address(0)) {
            revert InboxErrors.ZeroAddress();
        }

        localChainKey = chainKey;
        creditcoinChainId = creditcoinChainId_;
        defaultVoteValidator = initialValidator;
        bridgeReceiver = bridgeReceiver_;
    }

    function processedAt(
        bytes32 messageId
    ) external view override returns (uint256) {
        return _processedAt[messageId];
    }

    function isPending(
        bytes32 messageId
    ) external view override returns (bool) {
        return _isPending[messageId];
    }

    function validatedMessages(
        bytes32 messageId
    ) external view returns (bool) {
        return _validatedMessages[messageId];
    }

    function validationFailed(
        bytes32 messageId
    ) external view returns (bool) {
        return _validationFailed[messageId];
    }

    function deliverMessage(
        bytes32 messageId,
        address emitterAddress,
        bytes calldata payload,
        bytes calldata votes
    ) external override returns (bool success) {
        if (_processedAt[messageId] != 0) {
            revert InboxErrors.MessageAlreadyProcessed(messageId);
        }
        if (_validatedMessages[messageId]) {
            revert InboxErrors.MessageAlreadyValidated(messageId);
        }
        if (_validationFailed[messageId]) {
            revert InboxErrors.ValidationAlreadyFailed(messageId);
        }

        bytes32 messageHash = keccak256(
            abi.encode(
                messageId,
                emitterAddress,
                localChainKey,
                creditcoinChainId,
                payload
            )
        );

        bool ok = defaultVoteValidator.validateVotes(messageHash, votes);
        if (!ok) {
            _validationFailed[messageId] = true;
            emit ValidationFailed(messageId);
            return false;
        }

        emit ValidationSucceeded(messageId);

        _validatedMessages[messageId] = true;

        if (_deliver(messageId, emitterAddress, payload)) {
            _processedAt[messageId] = block.number;
            emit MessageDelivered(messageId, address(defaultVoteValidator));
        } else {
            _storePending(
                messageId,
                emitterAddress,
                payload
            );
            emit MessagePending(messageId, bridgeReceiver);
        }

        return true;
    }

    function retryPendingMessage(bytes32 messageId) external override {
        if (!_isPending[messageId]) {
            revert InboxErrors.MessageNotPending(messageId);
        }

        PendingMessage memory pending = _pendingMessages[messageId];

        bool delivered = _deliver(
            messageId,
            pending.emitterAddress,
            pending.payloadData
        );

        if (!delivered) {
            revert InboxErrors.RetryFailed(messageId);
        }

        delete _pendingMessages[messageId];
        _isPending[messageId] = false;

        _processedAt[messageId] = block.number;
        emit MessageDelivered(messageId, address(defaultVoteValidator));
    }

    function setDefaultVoteValidator(
        IVoteValidator newValidator
    ) external onlyOwner {
        if (address(newValidator) == address(0)) {
            revert InboxErrors.ZeroAddress();
        }

        address old = address(defaultVoteValidator);
        defaultVoteValidator = newValidator;
        emit DefaultVoteValidatorSet(old, address(newValidator));
    }

    function setBridgeReceiver(address receiver) external override onlyOwner {
        if (receiver == address(0)) {
            revert InboxErrors.ZeroAddress();
        }
        bridgeReceiver = receiver;
    }

    function _deliver(
        bytes32 messageId,
        address emitterAddress,
        bytes memory payloadData
    ) private returns (bool) {
        bytes memory callData = abi.encodeWithSelector(
            IMessageReceiver.receiveMessage.selector,
            messageId,
            creditcoinChainId,
            emitterAddress,
            payloadData
        );

        (bool success, ) = bridgeReceiver.call(callData);
        return success;
    }

    function _storePending(
        bytes32 messageId,
        address emitterAddress,
        bytes memory payload
    ) private {
        _isPending[messageId] = true;
        _pendingMessages[messageId] = PendingMessage({
            emitterAddress: emitterAddress,
            payloadData: payload
        });
    }
}
