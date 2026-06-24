// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/**
 * @title EvmV1Decoder
 * @notice Decodes ABI-encoded EVM transactions (types 0–4) and receipts from prover `txBytes`.
 * @dev Encoding: `abi.encode(uint8 txType, bytes[] chunks)` where each chunk groups fields
 *      to avoid "stack too deep" issues.
 *      - chunk[0]: common tx fields (nonce, gasLimit, from, toIsNull, to, value, data)
 *      - chunk[1]: type-specific fields (gasPrice / EIP-1559 params / access list / sigs)
 *      - chunk[last]: receipt fields (status, gasUsed, logs, logsBloom)
 *      Types 3 & 4 have an extra middle chunk (chunk[2]) for blob/auth list extras.
 *
 * Vendored from CCNext-smart-contracts (contracts/utility/EvmV1Decoder.sol), itself adapted from
 * usc-testnet-bridge-examples. Kept verbatim so it stays in sync with the prover's `txBytes`
 * encoding; only the receipt-log path (`decodeReceiptFields` + `getLogsByEventSignature`) is used
 * by the write-ability AcknowledgmentValidator.
 */
library EvmV1Decoder {
    struct AccessListEntry {
        address account;
        bytes32[] storageKeys;
    }

    struct AccessListEntryBytes32 {
        address account;
        bytes32[] storageKeys;
    }

    struct AccessListEntryUint256 {
        address account;
        uint256[] storageKeys;
    }

    struct AuthorizationListEntry {
        uint256 chainId;
        address account;
        uint64 nonce;
        uint8 yParity;
        uint256 r;
        uint256 s;
    }

    struct LogEntry {
        address address_;
        bytes32[] topics;
        bytes data;
    }

    struct LogEntryTuple {
        address address_;
        bytes32[] topics;
        bytes data;
    }

    /// @notice Fields shared across all EVM tx types.
    struct CommonTxFields {
        uint64 nonce;
        uint64 gasLimit;
        address from;
        bool toIsNull;
        address to;
        uint256 value;
        bytes data;
    }

    /// @notice Receipt fields shared across all EVM tx types.
    struct ReceiptFields {
        uint8 receiptStatus;
        uint64 receiptGasUsed;
        LogEntry[] receiptLogs;
        bytes receiptLogsBloom;
    }

    struct LegacyFields {
        uint128 gasPrice;
        uint256 v;
        bytes32 r;
        bytes32 s;
    }

    struct Type1Fields {
        uint64 chainId;
        uint128 gasPrice;
        AccessListEntry[] accessList;
        uint8 yParity;
        bytes32 r;
        bytes32 s;
    }

    struct Type2Fields {
        uint64 chainId;
        uint128 maxPriorityFeePerGas;
        uint128 maxFeePerGas;
        AccessListEntry[] accessList;
        uint8 yParity;
        bytes32 r;
        bytes32 s;
    }

    struct Type3Fields {
        uint64 chainId;
        uint128 maxPriorityFeePerGas;
        uint128 maxFeePerGas;
        AccessListEntry[] accessList;
        uint256 maxFeePerBlobGas;
        bytes32[] blobVersionedHashes;
        uint8 yParity;
        bytes32 r;
        bytes32 s;
    }

    struct Type4Fields {
        uint64 chainId;
        uint128 maxPriorityFeePerGas;
        uint128 maxFeePerGas;
        AccessListEntry[] accessList;
        AuthorizationListEntry[] authorizationList;
        uint8 yParity;
        bytes32 r;
        bytes32 s;
    }

    struct DecodedTransactionType0 {
        CommonTxFields commonTx;
        LegacyFields type0;
        ReceiptFields receipt;
    }

    struct DecodedTransactionType1 {
        CommonTxFields commonTx;
        Type1Fields type1;
        ReceiptFields receipt;
    }

    struct DecodedTransactionType2 {
        CommonTxFields commonTx;
        Type2Fields type2;
        ReceiptFields receipt;
    }

    struct DecodedTransactionType3 {
        CommonTxFields commonTx;
        Type3Fields type3;
        ReceiptFields receipt;
    }

    struct DecodedTransactionType4 {
        CommonTxFields commonTx;
        Type4Fields type4;
        ReceiptFields receipt;
    }

    /// @notice Extracts the tx type byte from encoded `(uint8, bytes[])` without full decode.
    function getTransactionType(bytes memory encodedTx) internal pure returns (uint8 txType) {
        assembly {
            txType := byte(31, mload(add(encodedTx, 32)))
        }
    }

    function isValidTransactionType(uint8 txType) internal pure returns (bool) {
        return txType <= 4;
    }

    /// @notice Filter logs by event signature (first topic).
    function getLogsByEventSignature(ReceiptFields memory receipt, bytes32 eventSignature)
        internal
        pure
        returns (LogEntry[] memory)
    {
        return getLogsByEventSignature(receipt.receiptLogs, eventSignature);
    }

    /// @notice Filter a log array by event signature.
    function getLogsByEventSignature(LogEntry[] memory logs, bytes32 eventSignature)
        internal
        pure
        returns (LogEntry[] memory)
    {
        uint256 n;
        for (uint256 i; i < logs.length; i++) {
            if (logs[i].topics.length > 0 && logs[i].topics[0] == eventSignature) n++;
        }
        LogEntry[] memory out = new LogEntry[](n);
        uint256 k;
        for (uint256 i; i < logs.length; i++) {
            if (logs[i].topics.length > 0 && logs[i].topics[0] == eventSignature) {
                out[k++] = logs[i];
            }
        }
        return out;
    }

    /// @notice Decode only the common tx fields (chunk 0). Does not decode receipt or type-specific fields.
    function decodeCommonTxFields(bytes memory encodedTx)
        internal
        pure
        returns (CommonTxFields memory common)
    {
        require(encodedTx.length > 0, "EvmV1Decoder: Empty");
        return _decodeCommonTxChunk(encodedTx);
    }

    /// @notice Decode only the receipt fields (last chunk). Most useful for log-based readability.
    function decodeReceiptFields(bytes memory encodedTx)
        internal
        pure
        returns (ReceiptFields memory)
    {
        require(encodedTx.length > 0, "EvmV1Decoder: Empty");
        uint8 txType = getTransactionType(encodedTx);
        require(txType <= 4, "EvmV1Decoder: Invalid tx type");
        return _decodeReceiptChunk(encodedTx, txType);
    }

    function decodeTransactionType0(bytes memory chunk)
        internal
        pure
        returns (DecodedTransactionType0 memory d)
    {
        require(chunk.length > 0, "EvmV1Decoder: Empty");
        (uint8 decodedTxType, bytes[] memory chunks) = abi.decode(chunk, (uint8, bytes[]));
        require(decodedTxType == 0, "EvmV1Decoder: Expected type 0");
        require(chunks.length == 3, "EvmV1Decoder: Wrong chunk count for type 0");
        d.commonTx = _decodeCommonTxChunk(chunk);
        d.type0 = _decodeTypeSpecificChunkType0(chunks[1]);
        d.receipt = _decodeReceiptChunk(chunk, 0);
    }

    function decodeTransactionType2(bytes memory chunk)
        internal
        pure
        returns (DecodedTransactionType2 memory d)
    {
        require(chunk.length > 0, "EvmV1Decoder: Empty");
        (uint8 decodedTxType, bytes[] memory chunks) = abi.decode(chunk, (uint8, bytes[]));
        require(decodedTxType == 2, "EvmV1Decoder: Expected type 2");
        require(chunks.length == 3, "EvmV1Decoder: Wrong chunk count for type 2");
        d.commonTx = _decodeCommonTxChunk(chunk);
        d.type2 = _decodeTypeSpecificChunkType2(chunks[1]);
        d.receipt = _decodeReceiptChunk(chunk, 2);
    }

    function _decodeCommonTxChunk(bytes memory chunk)
        internal
        pure
        returns (CommonTxFields memory common)
    {
        (, bytes[] memory chunks) = abi.decode(chunk, (uint8, bytes[]));
        require(chunks.length == 3 || chunks.length == 4, "EvmV1Decoder: Wrong chunk count");
        (
            common.nonce,
            common.gasLimit,
            common.from,
            common.toIsNull,
            common.to,
            common.value,
            common.data
        ) = abi.decode(chunks[0], (uint64, uint64, address, bool, address, uint256, bytes));
    }

    function _decodeReceiptChunk(bytes memory chunk, uint8 txType)
        internal
        pure
        returns (ReceiptFields memory receipt)
    {
        (, bytes[] memory chunks) = abi.decode(chunk, (uint8, bytes[]));

        uint256 receiptIdx = (txType <= 2) ? 2 : 3;
        if (txType <= 2) {
            require(chunks.length == 3, "EvmV1Decoder: Wrong chunk count for type 0-2");
        } else {
            require(chunks.length == 4, "EvmV1Decoder: Wrong chunk count for type 3-4");
        }

        uint8 receiptStatus;
        uint64 receiptGasUsed;
        LogEntryTuple[] memory logs;
        bytes memory logsBloom;

        (receiptStatus, receiptGasUsed, logs, logsBloom) =
            abi.decode(chunks[receiptIdx], (uint8, uint64, LogEntryTuple[], bytes));

        receipt.receiptStatus = receiptStatus;
        receipt.receiptGasUsed = receiptGasUsed;
        receipt.receiptLogs = _toLogs(logs);
        receipt.receiptLogsBloom = logsBloom;
    }

    function _decodeTypeSpecificChunkType0(bytes memory chunk)
        private
        pure
        returns (LegacyFields memory f)
    {
        (f.gasPrice, f.v, f.r, f.s) = abi.decode(chunk, (uint128, uint256, bytes32, bytes32));
    }

    function _decodeTypeSpecificChunkType2(bytes memory chunk)
        private
        pure
        returns (Type2Fields memory f)
    {
        AccessListEntryBytes32[] memory al;
        (f.chainId, f.maxPriorityFeePerGas, f.maxFeePerGas, al, f.yParity, f.r, f.s) = abi.decode(
            chunk, (uint64, uint128, uint128, AccessListEntryBytes32[], uint8, bytes32, bytes32)
        );
        f.accessList = _convertAccessListBytes32(al);
    }

    function _toLogs(LogEntryTuple[] memory t) private pure returns (LogEntry[] memory out) {
        out = new LogEntry[](t.length);
        for (uint256 i; i < t.length; i++) {
            out[i] = LogEntry({address_: t[i].address_, topics: t[i].topics, data: t[i].data});
        }
    }

    function _convertAccessListBytes32(AccessListEntryBytes32[] memory src)
        private
        pure
        returns (AccessListEntry[] memory dst)
    {
        dst = new AccessListEntry[](src.length);
        for (uint256 i; i < src.length; i++) {
            dst[i] = AccessListEntry({account: src[i].account, storageKeys: src[i].storageKeys});
        }
    }
}
