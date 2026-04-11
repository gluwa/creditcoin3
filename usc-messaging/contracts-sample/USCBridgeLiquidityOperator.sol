// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {
    Ownable2Step,
    Ownable
} from "@openzeppelin/contracts/access/Ownable2Step.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

import {IUSCBridgeLiquidityOperator} from "./abstract/IUSCBridgeLiquidityOperator.sol";
import {IERC20Burnable} from "./abstract/IERC20MintBurn.sol";
import {IOutbox} from "./abstract/IOutbox.sol";
import {IUSCProofVerifier} from "./abstract/IUSCProofVerifier.sol";
import {IUSCQuoteValidator} from "./abstract/IUSCQuoteValidator.sol";
import {IBridgeIntentDecoder} from "./abstract/IBridgeIntentDecoder.sol";
import {BridgeMessageCodecV1} from "./common/BridgeMessageCodecV1.sol";
import {BlockProverTypes} from "./common/BlockProverTypes.sol";
import {CrossChainOrderTypes} from "./common/CrossChainOrderTypes.sol";
import {USCBridgeTypes} from "./common/USCBridgeTypes.sol";
import {USCBridgeLiquidityOperatorErrors} from "./error/USCBridgeLiquidityOperatorErrors.sol";

contract USCBridgeLiquidityOperator is
    IUSCBridgeLiquidityOperator,
    Ownable2Step
{
    /// @dev Per-chain outbound transport configuration on Creditcoin.
    struct ChainConfig {
        IOutbox outbox;
        bool requiresAck;
        bool enabled;
    }

    event ChainConfigSet(
        bytes32 indexed chainKey,
        address indexed outbox,
        bool requiresAck,
        bool enabled
    );
    event ChainConfigRemoved(bytes32 indexed chainKey);
    event ProofVerifierSet(address indexed verifier);
    event QuoteValidatorSet(address indexed validator);
    event BridgeIntentDecoderSet(address indexed decoder);
    event BridgeIntentExecutionTracked(
        bytes32 indexed intentId,
        bool success,
        uint256 gasUsed,
        bytes32 returnedDataHash
    );

    IERC20 public immutable token;
    bool public immutable isHubAndSpoke;
    bool public immutable isCreditcoinHub;

    uint256 public intentNonce;
    mapping(bytes32 => ChainConfig) private _chainConfigs;
    mapping(bytes32 => bool) public processedIntentIds;

    IUSCProofVerifier public proofVerifier;
    IUSCQuoteValidator public quoteValidator;
    IBridgeIntentDecoder public bridgeIntentDecoder;

    /// @dev Outbound message kind: PayloadOnly, TokenOnly, TokenAndPayload.
    enum OutboundMessageKind { PayloadOnly, TokenOnly, TokenAndPayload }

    /// @param tokenAddress_ ERC20 token managed by this bridge operator.
    /// @param isHubAndSpoke_ True for lock/unlock model, false for burn/mint model.
    /// @param isCreditcoinHub_ True if Creditcoin is the lock/unlock hub.
    /// @param initialOwner_ Contract owner with admin permissions.
    constructor(
        address tokenAddress_,
        bool isHubAndSpoke_,
        bool isCreditcoinHub_,
        address initialOwner_
    ) Ownable(initialOwner_) {
        if (tokenAddress_ == address(0) || initialOwner_ == address(0)) {
            revert USCBridgeLiquidityOperatorErrors.ZeroAddress();
        }

        token = IERC20(tokenAddress_);
        isHubAndSpoke = isHubAndSpoke_;
        isCreditcoinHub = isCreditcoinHub_;
    }

    /// @notice Sets outbound chain configuration used by `bridgeTo`.
    function setChainConfig(
        bytes32 chainKey,
        address outbox,
        bool requiresAck,
        bool enabled
    ) external onlyOwner {
        if (outbox == address(0)) {
            revert USCBridgeLiquidityOperatorErrors.ZeroAddress();
        }

        _chainConfigs[chainKey] = ChainConfig({
            outbox: IOutbox(outbox),
            requiresAck: requiresAck,
            enabled: enabled
        });
        emit ChainConfigSet(chainKey, outbox, requiresAck, enabled);
    }

    function removeChainConfig(bytes32 chainKey) external onlyOwner {
        delete _chainConfigs[chainKey];
        emit ChainConfigRemoved(chainKey);
    }

    function getChainConfig(
        bytes32 chainKey
    ) external view returns (address outbox, bool requiresAck, bool enabled) {
        ChainConfig memory config = _chainConfigs[chainKey];
        return (address(config.outbox), config.requiresAck, config.enabled);
    }

    function setProofVerifier(address verifier) external onlyOwner {
        if (verifier == address(0)) {
            revert USCBridgeLiquidityOperatorErrors.ZeroAddress();
        }
        proofVerifier = IUSCProofVerifier(verifier);
        emit ProofVerifierSet(verifier);
    }

    function setQuoteValidator(address validator) external onlyOwner {
        if (validator == address(0)) {
            revert USCBridgeLiquidityOperatorErrors.ZeroAddress();
        }
        quoteValidator = IUSCQuoteValidator(validator);
        emit QuoteValidatorSet(validator);
    }

    function setBridgeIntentDecoder(address decoder) external onlyOwner {
        if (decoder == address(0)) {
            revert USCBridgeLiquidityOperatorErrors.ZeroAddress();
        }
        bridgeIntentDecoder = IBridgeIntentDecoder(decoder);
        emit BridgeIntentDecoderSet(decoder);
    }

    /// @notice Initiates Creditcoin -> client-chain bridge by publishing through Outbox.
    /// @dev The outbound payload uses canonical V1 codec format.
    function bridgeTo(
        bytes32 chainKey,
        USCBridgeTypes.BridgeMessage calldata message,
        bytes calldata quote
    ) external payable override {
        ChainConfig memory config = _chainConfigs[chainKey];
        if (address(config.outbox) == address(0)) {
            revert USCBridgeLiquidityOperatorErrors.ChainKeyNotConfigured(
                chainKey
            );
        }
        if (!config.enabled) {
            revert USCBridgeLiquidityOperatorErrors.ChainKeyDisabled(chainKey);
        }
        if (address(quoteValidator) == address(0)) {
            revert USCBridgeLiquidityOperatorErrors.MissingAdapterConfig();
        }
        if (message.receiver.length == 0) {
            revert USCBridgeLiquidityOperatorErrors.EmptyReceiver();
        }

        bool hasTokenTransfer = _hasTokenTransfer(message.tokenAmount);
        if (!hasTokenTransfer && message.data.length == 0) {
            revert USCBridgeLiquidityOperatorErrors.EmptyBridgeMessage();
        }

        bytes32 messageHash = keccak256(
            abi.encode(
                message.receiver,
                message.data,
                message.tokenAmount.token,
                message.tokenAmount.amount,
                message.gasLimit
            )
        );
        bool quoteValid = quoteValidator.validateQuote{value: msg.value}(
            chainKey,
            msg.sender,
            messageHash,
            quote
        );
        if (!quoteValid) {
            revert USCBridgeLiquidityOperatorErrors.QuoteValidationFailed();
        }

        if (hasTokenTransfer) {
            token.transferFrom(
                msg.sender,
                address(this),
                message.tokenAmount.amount
            );

            if (!isHubAndSpoke || !isCreditcoinHub) {
                IERC20Burnable(address(token)).burn(message.tokenAmount.amount);
            }
        }

        uint256 nonce = intentNonce++;
        uint8 msgType = _deriveOutboundMsgType(
            hasTokenTransfer,
            message.data.length > 0
        );
        bytes32 intentId = keccak256(
            abi.encodePacked(msgType, nonce, chainKey)
        );

        BridgeMessageCodecV1.BridgePayloadV1 memory payload = BridgeMessageCodecV1
            .BridgePayloadV1({intentId: intentId, message: message});

        config.outbox.publishMessage(
            config.requiresAck,
            BridgeMessageCodecV1.encode(payload)
        );
        emit BridgeIntent(intentId, chainKey, message);
    }

    /// @notice Processes one inbound intent decoded from a proved source transaction.
    function bridgeFromIntent(
        bytes32 chainKey,
        uint64 blockHeight,
        bytes calldata encodedTransaction,
        BlockProverTypes.InclusionProof calldata inclusionProof,
        BlockProverTypes.ContinuityProof calldata continuityProof
    )
        external
        override
        returns (bool isValid, bytes[] memory extractedTransactionData)
    {
        _validateInboundInputs(chainKey, blockHeight, encodedTransaction);
        _verifyProofs(
            chainKey,
            blockHeight,
            encodedTransaction,
            inclusionProof,
            continuityProof
        );

        IBridgeIntentDecoder.DecodedBridgeIntent memory decoded = bridgeIntentDecoder
            .decodeBridgeIntent(encodedTransaction);
        CrossChainOrderTypes.CrossChainOrder memory order = decoded.order;
        USCBridgeTypes.AttestedTxData memory decodedAttestedTxData = decoded
            .attestedTxData;

        bytes32 intentId = keccak256(
            abi.encodePacked(
                chainKey,
                order.nonce,
                order.user
            )
        );
        CrossChainOrderTypes.CrossChainIntent memory intent = _decodeIntent(
            order.orderData
        );
        extractedTransactionData = _buildExtractedTransactionData(
            decodedAttestedTxData,
            intent,
            intentId
        );
        if (processedIntentIds[intentId]) {
            return (false, extractedTransactionData);
        }
        if (!_validateMintIntentAndMatchAttestation(order, intent, decodedAttestedTxData)) {
            return (false, extractedTransactionData);
        }
        isValid = _processValidatedIntent(chainKey, intentId, order, intent);
        return (isValid, extractedTransactionData);
    }

    function _hasTokenTransfer(
        USCBridgeTypes.EVMTokenAmount calldata evmTokenAmount
    ) internal view returns (bool) {
        if (evmTokenAmount.token == address(0)) {
            if (evmTokenAmount.amount != 0) {
                revert USCBridgeLiquidityOperatorErrors.InvalidTokenAmount();
            }
            return false;
        }
        if (evmTokenAmount.token != address(token)) {
            revert USCBridgeLiquidityOperatorErrors.InvalidTokenAddress(
                evmTokenAmount.token
            );
        }
        if (evmTokenAmount.amount == 0) {
            revert USCBridgeLiquidityOperatorErrors.InvalidTokenAmount();
        }
        return true;
    }

    function _validateInboundInputs(
        bytes32 chainKey,
        uint64 blockHeight,
        bytes calldata encodedTransaction
    ) internal view {
        ChainConfig memory config = _chainConfigs[chainKey];
        if (address(config.outbox) == address(0)) {
            revert USCBridgeLiquidityOperatorErrors.ChainKeyNotConfigured(
                chainKey
            );
        }
        if (!config.enabled) {
            revert USCBridgeLiquidityOperatorErrors.ChainKeyDisabled(chainKey);
        }
        if (blockHeight == 0) {
            revert USCBridgeLiquidityOperatorErrors.InvalidBlockHeight();
        }
        if (encodedTransaction.length == 0) {
            revert USCBridgeLiquidityOperatorErrors.EmptyEncodedTransaction();
        }
        if (
            address(proofVerifier) == address(0) ||
            address(bridgeIntentDecoder) == address(0)
        ) {
            revert USCBridgeLiquidityOperatorErrors.MissingAdapterConfig();
        }
    }

    function _verifyProofs(
        bytes32 chainKey,
        uint64 blockHeight,
        bytes calldata encodedTransaction,
        BlockProverTypes.InclusionProof calldata inclusionProof,
        BlockProverTypes.ContinuityProof calldata continuityProof
    ) internal view {
        bool valid = proofVerifier.verifyProofs(
            chainKey,
            blockHeight,
            encodedTransaction,
            inclusionProof,
            continuityProof
        );
        if (!valid) {
            revert USCBridgeLiquidityOperatorErrors.ProofValidationFailed();
        }
    }

    function _callDestination(
        address receiver,
        bytes memory callData
    ) internal returns (bool success, bytes memory retData, uint256 gasUsed) {
        uint256 gasBefore = gasleft();
        (success, retData) = receiver.call(callData);
        gasUsed = gasBefore - gasleft();
    }

    function _validateMintIntentAndMatchAttestation(
        CrossChainOrderTypes.CrossChainOrder memory order,
        CrossChainOrderTypes.CrossChainIntent memory intent,
        USCBridgeTypes.AttestedTxData memory attestedTxData
    ) internal view returns (bool) {
        if (order.user == address(0) || order.originSettler == address(0)) {
            return false;
        }
        if (order.openDeadline != 0 && block.timestamp < order.openDeadline) {
            return false;
        }
        if (order.fillDeadline != 0 && block.timestamp > order.fillDeadline) {
            return false;
        }
        if (
            order.openDeadline != 0 &&
            order.fillDeadline != 0 &&
            order.openDeadline > order.fillDeadline
        ) {
            return false;
        }
        if (intent.action != CrossChainOrderTypes.OrderAction.MINT) {
            return false;
        }
        if (intent.minDestinationAmount.token != address(token)) {
            return false;
        }
        if (intent.minDestinationAmount.amount == 0) {
            return false;
        }
        if (
            intent.destinationChainId != block.chainid
        ) {
            return false;
        }
        if (intent.sourceChainId != order.originChainId) {
            return false;
        }
        if (
            intent.sourceProofRequirement != bytes32(0) &&
            intent.sourceProofRequirement != attestedTxData.proofContextHash
        ) {
            return false;
        }
        if (order.user != attestedTxData.user) {
            return false;
        }
        if (order.nonce != attestedTxData.nonce) {
            return false;
        }
        if (order.originChainId != attestedTxData.sourceChainId) {
            return false;
        }
        if (intent.sourceAmount.token != attestedTxData.sourceAmount.token) {
            return false;
        }
        if (intent.sourceAmount.amount != attestedTxData.sourceAmount.amount) {
            return false;
        }
        if (intent.recipient == address(0) || intent.destinationContract == address(0)) {
            return false;
        }
        if (intent.destinationCallData.length == 0) {
            return false;
        }
        return true;
    }

    function _decodeIntent(
        bytes memory orderData
    )
        internal
        pure
        returns (CrossChainOrderTypes.CrossChainIntent memory intent)
    {
        if (orderData.length == 0) {
            revert USCBridgeLiquidityOperatorErrors.InvalidIntentOrderData();
        }
        intent = abi.decode(orderData, (CrossChainOrderTypes.CrossChainIntent));
    }

    function _buildExtractedTransactionData(
        USCBridgeTypes.AttestedTxData memory attestedTxData,
        CrossChainOrderTypes.CrossChainIntent memory intent,
        bytes32 intentId
    ) internal pure returns (bytes[] memory extractedTransactionData) {
        // Extracted transaction data schema:
        // [0] abi.encode(uint8 version)
        // [1] abi.encode(USCBridgeTypes.AttestedTxData)
        // [2] abi.encode(CrossChainOrderTypes.CrossChainIntent)
        // [3] abi.encode(bytes32 intentId)
        extractedTransactionData = new bytes[](4);
        uint8 version = 1;
        extractedTransactionData[0] = abi.encode(version);
        extractedTransactionData[1] = abi.encode(attestedTxData);
        extractedTransactionData[2] = abi.encode(intent);
        extractedTransactionData[3] = abi.encode(intentId);
    }

    function _processValidatedIntent(
        bytes32 chainKey,
        bytes32 intentId,
        CrossChainOrderTypes.CrossChainOrder memory order,
        CrossChainOrderTypes.CrossChainIntent memory intent
    ) internal returns (bool) {
        (bool success, bytes memory retData, uint256 gasUsed) = _callDestination(
            intent.destinationContract,
            intent.destinationCallData
        );
        emit BridgeIntentExecutionTracked(
            intentId,
            success,
            gasUsed,
            keccak256(retData)
        );
        if (!success) {
            return false;
        }
        // TODO: check maxGasCost?

        processedIntentIds[intentId] = true;
        emit CrossChainOrderProcessed(intentId, chainKey, order);
        return true;
    }

    function _deriveOutboundMsgType(
        bool hasTokenTransfer,
        bool hasPayload
    ) internal pure returns (uint8 msgType) {
        if (hasTokenTransfer && hasPayload) {
            return uint8(OutboundMessageKind.TokenAndPayload);
        }
        if (hasTokenTransfer) {
            return uint8(OutboundMessageKind.TokenOnly);
        }
        return uint8(OutboundMessageKind.PayloadOnly);
    }

}
