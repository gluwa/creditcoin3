// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {IBridgeIntentDecoder} from "../abstract/IBridgeIntentDecoder.sol";
import {CrossChainOrderTypes} from "../common/CrossChainOrderTypes.sol";
import {USCBridgeTypes} from "../common/USCBridgeTypes.sol";

contract MockBridgeIntentDecoder is IBridgeIntentDecoder {
    mapping(bytes32 => bytes) private _decodedPayloads;

    function setDecodedBridgeIntent(
        bytes calldata encodedTransaction,
        CrossChainOrderTypes.CrossChainOrder calldata order
    ) external {
        DecodedBridgeIntent memory decoded = DecodedBridgeIntent({
            order: order,
            attestedTxData: _deriveAttestedTxData(encodedTransaction, order)
        });
        _decodedPayloads[keccak256(encodedTransaction)] = abi.encode(decoded);
    }

    function setDecodedAttestedTxData(
        bytes calldata encodedTransaction,
        USCBridgeTypes.AttestedTxData calldata attestedTxData
    ) external {
        bytes32 txKey = keccak256(encodedTransaction);
        bytes memory encoded = _decodedPayloads[txKey];
        if (encoded.length == 0) {
            return;
        }
        DecodedBridgeIntent memory decoded = abi.decode(
            encoded,
            (DecodedBridgeIntent)
        );
        decoded.attestedTxData = attestedTxData;
        _decodedPayloads[txKey] = abi.encode(decoded);
    }

    function decodeBridgeIntent(
        bytes calldata encodedTransaction
    ) external view returns (DecodedBridgeIntent memory) {
        bytes memory encoded = _decodedPayloads[keccak256(encodedTransaction)];
        if (encoded.length == 0) {
            return
                DecodedBridgeIntent({
                    order: CrossChainOrderTypes.CrossChainOrder({
                        originSettler: address(0),
                        user: address(0),
                        nonce: 0,
                        originChainId: 0,
                        openDeadline: 0,
                        fillDeadline: 0,
                        orderData: ""
                    }),
                    attestedTxData: USCBridgeTypes.AttestedTxData({
                        user: address(0),
                        nonce: 0,
                        sourceChainId: 0,
                        sourceAmount: USCBridgeTypes.EVMTokenAmount({
                            token: address(0),
                            amount: 0
                        }),
                        proofContextHash: bytes32(0)
                    })
                });
        }
        return abi.decode(encoded, (DecodedBridgeIntent));
    }

    function _deriveAttestedTxData(
        bytes calldata encodedTransaction,
        CrossChainOrderTypes.CrossChainOrder calldata order
    ) internal pure returns (USCBridgeTypes.AttestedTxData memory attestedTxData) {
        USCBridgeTypes.EVMTokenAmount memory sourceAmount;
        if (order.orderData.length > 0) {
            CrossChainOrderTypes.CrossChainIntent memory intent = abi.decode(
                order.orderData,
                (CrossChainOrderTypes.CrossChainIntent)
            );
            sourceAmount = intent.sourceAmount;
        }

        attestedTxData = USCBridgeTypes.AttestedTxData({
            user: order.user,
            nonce: order.nonce,
            sourceChainId: order.originChainId,
            sourceAmount: sourceAmount,
            proofContextHash: keccak256(encodedTransaction)
        });
    }
}
