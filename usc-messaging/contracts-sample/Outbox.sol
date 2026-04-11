// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {
    Ownable2Step,
    Ownable
} from "@openzeppelin/contracts/access/Ownable2Step.sol";

import {OutboxTypes} from "./common/OutboxTypes.sol";
import {OutboxErrors} from "./error/OutboxErrors.sol";
import {UniversalContract} from "./abstract/UniversalContract.sol";
import {Storage} from "./common/Storage.sol";
import {RateLimitLib} from "./common/RateLimitLib.sol";
import {IOutbox} from "./abstract/IOutbox.sol";

contract Outbox is IOutbox, Storage, Ownable2Step {
    using OutboxTypes for *;
    using RateLimitLib for RateLimitLib.RateBucket;

    bool private _initialized;

    /// @dev we will make validator as a contract interface later
    address private _validator;
    bytes32 private _defaultRateLimit;
    mapping(address => uint64) _sequence;

    /// @dev per-dApp rate limit overrides
    mapping(address => uint128) public rateLimitOverride;


    modifier onlyValidator() {
        if (msg.sender != _state().validator) {
            revert OutboxErrors.NotValidator();
        }
        _;
    }

    modifier initializer() {
        if (_initialized) {
            revert OutboxErrors.AlreadyInitialized();
        }
        _;
        _initialized = true;
    }

    constructor(
        uint32 chainKey,
        address initialOwner,
        address initialValidator,
        uint128 initialRateLimit
    ) Ownable(initialOwner) {
        if (initialValidator == address(0)) {
            revert OutboxErrors.ZeroAddress();
        }

        OutboxState storage s = _state();
        s.chainKey = chainKey;
        s.evmChainId = block.chainid;
        s.validator = initialValidator;
        s.defaultRateLimit = initialRateLimit;
    }

    function owner() public view override(IOutbox, Ownable) returns (address) {
        return Ownable.owner();
    }

    function validator() external view override returns (address) {
        return _state().validator;
    }

    function defaultRateLimit() external view override returns (uint128) {
        return _state().defaultRateLimit;
    }

    function getSequence(
        address usContract
    ) external view override returns (uint64) {
        return _state().ucSequences[usContract];
    }

    function getMessage(
        bytes32 messageId
    ) external view override returns (OutboxTypes.Message memory m) {
        m = _state().messages[messageId];
        if (m.emitter == address(0)) {
            revert OutboxErrors.MessageNotFound(messageId);
        }
    }

    function isAcknowledged(
        bytes32 messageId
    ) external view override returns (bool) {
        return _state().messages[messageId].acknowledged;
    }

    function messageRequiresAck(
        bytes32 messageId
    ) external view override returns (bool) {
        return _state().messages[messageId].requiresAck;
    }

    function publishMessage(
        bool requiresAck,
        bytes calldata payload
    ) external override returns (bytes32 messageId) {
        OutboxState storage s = _state();

        s.rateBuckets[msg.sender].enforce(s.defaultRateLimit);

        address usContract = msg.sender;

        uint64 seq = uint64(++s.ucSequences[usContract]);

        bytes32 payloadHash = keccak256(payload);

        messageId = OutboxTypes.computeMessageId(
            address(this),
            usContract,
            seq,
            payloadHash
        );

        s.messages[messageId] = OutboxTypes.Message({
            emitter: usContract,
            sequence: seq,
            timestamp: uint64(block.timestamp),
            requiresAck: requiresAck,
            acknowledged: false,
            payloadHash: payloadHash
        });

        emit MessagePublished(
            messageId,
            bytes32(bytes20(usContract)),
            requiresAck,
            payload
        );
    }

    function acknowledgeMessage(
        bytes32 messageId
    ) public override onlyValidator {
        OutboxState storage s = _state();
        OutboxTypes.Message storage m = s.messages[messageId];

        if (m.emitter == address(0)) {
            revert OutboxErrors.MessageNotFound(messageId);
        }
        if (!m.requiresAck) {
            revert OutboxErrors.MessageDoesNotRequireAck(messageId);
        }
        if (m.acknowledged) {
            revert OutboxErrors.MessageAlreadyAcknowledged(messageId);
        }

        m.acknowledged = true;
        emit MessageAcknowledged(messageId);
    }

    function batchAcknowledgeMessages(
        bytes32[] calldata messageIds
    ) external override onlyValidator {
        for (uint256 i = 0; i < messageIds.length; ++i) {
            acknowledgeMessage(messageIds[i]);
        }
    }

    function setValidator(address newValidator) external override onlyOwner {
        if (newValidator == address(0)) {
            revert OutboxErrors.ZeroAddress();
        }

        OutboxState storage s = _state();
        address old = s.validator;
        s.validator = newValidator;

        emit ValidatorChanged(old, newValidator);
    }
}
