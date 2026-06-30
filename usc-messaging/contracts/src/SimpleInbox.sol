// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IVoteValidator} from "./IVoteValidator.sol";

/// @notice Simple inbox for PoC. Accepts messages and delivers to destination contract.
contract SimpleInbox {
    IVoteValidator public voteValidator;
    uint256 public immutable creditcoinChainId;
    bytes32 public immutable localChainKey;

    bytes4 constant RECEIVE_SELECTOR =
        bytes4(keccak256("receiveMessage(bytes32,uint256,address,bytes)"));

    mapping(bytes32 => bool) private validatedMessages;

    struct PendingMessage {
        address destinationContract;
        address emitterAddress;
        bytes payload;
    }

    mapping(bytes32 => PendingMessage) private pendingMessages;
    mapping(bytes32 => bool) public isPending;

    event MessageDelivered(bytes32 indexed messageId);
    event MessagePending(bytes32 indexed messageId, address indexed destinationContract);

    constructor(address _voteValidator, uint256 _creditcoinChainId, bytes32 _localChainKey) {
        require(_voteValidator != address(0), "Invalid validator");
        voteValidator = IVoteValidator(_voteValidator);
        creditcoinChainId = _creditcoinChainId;
        localChainKey = _localChainKey;
    }

    /// @notice The exact digest attestors sign and `deliverMessage` recomputes before validating
    /// votes: `keccak256(abi.encode(messageId, emitterAddress, localChainKey, creditcoinChainId,
    /// payload))` (raw 32-byte hash, no EIP-191 prefix). Exposed as a view so off-chain components
    /// (the Rust attestor / relayer) can assert byte-for-byte parity against this contract instead
    /// of mirroring the formula — any drift would silently break delivery.
    function computeMessageHash(bytes32 messageId, address emitterAddress, bytes calldata payload)
        public
        view
        returns (bytes32)
    {
        return
            keccak256(
                abi.encode(messageId, emitterAddress, localChainKey, creditcoinChainId, payload)
            );
    }

    function deliverMessage(
        bytes32 messageId,
        address emitterAddress,
        bytes calldata payload,
        bytes calldata votes
    ) external returns (bool) {
        require(!validatedMessages[messageId], "Already validated");

        bytes32 messageHash = computeMessageHash(messageId, emitterAddress, payload);
        voteValidator.validateVotes(messageHash, votes);

        validatedMessages[messageId] = true;

        (address destinationContract, bytes memory payloadData) =
            abi.decode(payload, (address, bytes));

        try this.executeDelivery(destinationContract, messageId, emitterAddress, payloadData) {
            emit MessageDelivered(messageId);
        } catch {
            isPending[messageId] = true;
            pendingMessages[messageId] =
                PendingMessage(destinationContract, emitterAddress, payloadData);
            emit MessagePending(messageId, destinationContract);
        }

        return true;
    }

    function retryPendingMessage(bytes32 messageId) external {
        require(isPending[messageId], "Not pending");
        PendingMessage memory p = pendingMessages[messageId];
        delete pendingMessages[messageId];
        isPending[messageId] = false; // Clear before call to prevent reentrancy
        this.executeDelivery(p.destinationContract, messageId, p.emitterAddress, p.payload);
        emit MessageDelivered(messageId);
    }

    function executeDelivery(
        address destinationContract,
        bytes32 messageId,
        address emitterAddress,
        bytes memory payload
    ) external {
        require(msg.sender == address(this), "Only self");
        (bool ok,) = destinationContract.call(
            abi.encodeWithSelector(
                RECEIVE_SELECTOR, messageId, creditcoinChainId, emitterAddress, payload
            )
        );
        require(ok, "Delivery failed");
    }
}
