//! OP-Stack specifics: the `0x7e` **deposit transaction**, its receipt, and their Merkle leaf.
//!
//! OP-Stack rollups (Base, OP Mainnet, …) are Ethereum-equivalent except for one extra
//! transaction type. Every L2 block opens with a deposit transaction that writes the L1 block
//! attributes into the `L1Block` predeploy, and user deposits through the `OptimismPortal` are
//! delivered the same way. Deposit transactions are *unsigned* — they are derived from L1 — and
//! carry three fields no L1 type has: `sourceHash`, `mint` and `isSystemTx`.
//!
//! alloy's `Ethereum` network types reject the `0x7e` type outright, so an attestor typed on them
//! cannot even parse a Base block. This module supplies everything the block pipeline needs to
//! treat deposit transactions as first-class citizens:
//!
//! - [`DepositTransaction`]: parsed from the `AnyNetwork` "unknown transaction" catch-all, with
//!   the EIP-2718 encoding needed to recompute the header's `transactionsRoot`. The encoding is
//!   self-checked against the RPC-reported hash, so a wrong field layout can never silently
//!   produce a bogus root.
//! - [`encode_deposit_receipt_2718`]: the receipt encoding used for `receiptsRoot`, including the
//!   post-Canyon `depositNonce` / `depositReceiptVersion` tail.
//! - [`abi_encode_deposit_leaf`]: the Merkle leaf for a (deposit tx, receipt) pair, in the same
//!   `abi.encode(uint8 txType, bytes[] chunks)` envelope the `usc-abi-encoding` crate produces
//!   for types `0x0`–`0x4`, with `txType = 126`.
//!
//! # Specification references
//!
//! - Deposit transaction type and RLP layout: OP-Stack specs, `deposits.md`
//!   (`0x7E || rlp([sourceHash, from, to, mint, value, gas, isSystemTx, data])`).
//! - Receipt encoding: `op-geth` `core/types/receipt.go`, `Receipts.EncodeIndex` — with
//!   `depositReceiptVersion` present (Canyon and later) the receipt RLP is
//!   `[status, cumulativeGasUsed, logsBloom, logs, depositNonce, depositReceiptVersion]`;
//!   before Canyon it is the plain four-field receipt.
//!
//! # Leaf layout (`txType = 126`)
//!
//! Three chunks, mirroring the V1 layout for types `0x0`–`0x2` so a decoder that already
//! understands the common and receipt chunks needs only the middle one:
//!
//! | chunk | fields |
//! |-------|--------|
//! | `chunks[0]` common | `(uint64 nonce, uint64 gasLimit, address from, bool toIsNull, address to, uint256 value, bytes data)` |
//! | `chunks[1]` deposit | `(bytes32 sourceHash, uint256 mint, bool isSystemTx)` |
//! | `chunks[2]` receipt | `(uint8 status, uint64 gasUsed, (address,bytes32[],bytes)[] logs, bytes logsBloom)` |
//!
//! `nonce` is the deposit nonce the sequencer assigned (reported as `nonce` on the RPC
//! transaction). There are no signature fields: deposits are unsigned.
//!
//! This layout is the source of truth until it is upstreamed into `usc-abi-encoding`; keep the
//! two in sync when that happens.

use alloy::{
    consensus::{Eip658Value, Receipt, ReceiptWithBloom},
    dyn_abi::DynSolValue,
    network::{AnyReceiptEnvelope, AnyRpcTransaction, AnyTxEnvelope, UnknownTxEnvelope},
    primitives::{keccak256, Address, Bytes, Log as PrimitiveLog, TxKind, B256, U256, U64},
    rlp::{BufMut, Encodable},
    rpc::types::{Log as RpcLog, TransactionReceipt},
};

/// EIP-2718 type byte of an OP-Stack deposit transaction.
pub const DEPOSIT_TX_TYPE: u8 = 0x7e;

/// Errors turning RPC payloads into [`DepositTransaction`]s.
#[derive(Debug, thiserror::Error)]
pub enum DepositError {
    /// The transaction is not a `0x7e` deposit.
    #[error("transaction {hash} has type {ty:#x}, not a deposit (0x7e)")]
    NotADeposit { hash: B256, ty: u8 },
    /// A required deposit field is absent or malformed on the RPC transaction object.
    #[error("deposit transaction {hash}: missing or malformed field `{field}`")]
    Field { hash: B256, field: &'static str },
    /// Our RLP encoding of the deposit does not hash to the hash the RPC reported. Either the
    /// provider is lying or the encoding here is wrong; both mean the block cannot be trusted.
    #[error(
        "deposit transaction hash mismatch: rpc reports {reported}, encoding hashes to {computed}"
    )]
    HashMismatch { reported: B256, computed: B256 },
    /// A deposit receipt reports a `depositReceiptVersion` but no `depositNonce`; op-geth never
    /// produces this shape.
    #[error("deposit receipt for {hash} has depositReceiptVersion but no depositNonce")]
    ReceiptMissingNonce { hash: B256 },
}

/// An OP-Stack deposit transaction as it appears in an L2 block.
///
/// Field names follow the OP-Stack spec. `nonce` is not part of the consensus encoding (it is
/// derived by the sequencer and surfaced on the RPC object) but it is what the leaf's common chunk
/// reports as `nonce`, so it is carried along.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepositTransaction {
    /// Transaction hash as reported by the RPC, verified against our own encoding.
    pub hash: B256,
    /// Uniquely identifies the L1 origin of this deposit (see spec: user-deposited vs
    /// L1-info deposits derive it differently).
    pub source_hash: B256,
    /// L2 sender. For the L1-attributes deposit this is the fixed depositor address.
    pub from: Address,
    /// Call target, or contract creation.
    pub to: TxKind,
    /// ETH minted on L2 as part of this deposit.
    pub mint: U256,
    /// ETH value transferred from `from` to `to`.
    pub value: U256,
    /// Gas limit.
    pub gas_limit: u64,
    /// Whether the deposit is exempt from L2 gas accounting (always `false` since Regolith, but
    /// still part of the encoding).
    pub is_system_tx: bool,
    /// Calldata.
    pub input: Bytes,
    /// Sequencer-assigned deposit nonce (RPC `nonce`).
    pub nonce: u64,
}

impl DepositTransaction {
    /// Parse a deposit from an `AnyNetwork` RPC transaction and verify its hash.
    ///
    /// Fails with [`DepositError::NotADeposit`] if the transaction is anything else, so callers
    /// can match on the type byte first without a second inspection.
    pub fn try_from_rpc(tx: &AnyRpcTransaction) -> Result<Self, DepositError> {
        let unknown = match &tx.inner.inner {
            AnyTxEnvelope::Unknown(unknown) if unknown.inner.ty.0 == DEPOSIT_TX_TYPE => unknown,
            AnyTxEnvelope::Unknown(unknown) => {
                return Err(DepositError::NotADeposit {
                    hash: unknown.hash,
                    ty: unknown.inner.ty.0,
                })
            }
            AnyTxEnvelope::Ethereum(envelope) => {
                return Err(DepositError::NotADeposit {
                    hash: *envelope.tx_hash(),
                    ty: envelope.tx_type() as u8,
                })
            }
        };
        Self::try_from_unknown(unknown, tx.inner.from)
    }

    /// Parse a deposit from the catch-all envelope plus the RPC-level `from`.
    pub fn try_from_unknown(
        unknown: &UnknownTxEnvelope,
        from: Address,
    ) -> Result<Self, DepositError> {
        let hash = unknown.hash;
        if unknown.inner.ty.0 != DEPOSIT_TX_TYPE {
            return Err(DepositError::NotADeposit {
                hash,
                ty: unknown.inner.ty.0,
            });
        }
        let fields = &unknown.inner.fields;
        let field = |name: &'static str| DepositError::Field { hash, field: name };

        let source_hash: B256 = fields
            .get_deserialized("sourceHash")
            .and_then(Result::ok)
            .ok_or(field("sourceHash"))?;
        // `mint` is omitted by some providers when zero; `isSystemTx` is omitted post-Regolith
        // by some providers as well. Both default to their zero value per the spec.
        let mint: U256 = match fields.get_deserialized::<U256>("mint") {
            Some(Ok(v)) => v,
            Some(Err(_)) => return Err(field("mint")),
            None => U256::ZERO,
        };
        let is_system_tx: bool = match fields.get_deserialized::<bool>("isSystemTx") {
            Some(Ok(v)) => v,
            Some(Err(_)) => return Err(field("isSystemTx")),
            None => false,
        };
        // Standard fields: `to` may be `null` for contract creation.
        let to = match fields.get_deserialized::<Option<Address>>("to") {
            Some(Ok(Some(addr))) => TxKind::Call(addr),
            Some(Ok(None)) | None => TxKind::Create,
            Some(Err(_)) => return Err(field("to")),
        };
        let value: U256 = fields
            .get_deserialized("value")
            .and_then(Result::ok)
            .ok_or(field("value"))?;
        let gas_limit: u64 = fields
            .get_deserialized::<U64>("gas")
            .and_then(Result::ok)
            .ok_or(field("gas"))?
            .to();
        let input: Bytes = fields
            .get_deserialized("input")
            .and_then(Result::ok)
            .ok_or(field("input"))?;
        let nonce: u64 = fields
            .get_deserialized::<U64>("nonce")
            .and_then(Result::ok)
            .ok_or(field("nonce"))?
            .to();

        let deposit = Self {
            hash,
            source_hash,
            from,
            to,
            mint,
            value,
            gas_limit,
            is_system_tx,
            input,
            nonce,
        };

        // Self-check: the EIP-2718 encoding must hash to what the node reported. This is what
        // lets us trust the recomputed `transactionsRoot` downstream.
        let computed = keccak256(deposit.encoded_2718());
        if computed != hash {
            return Err(DepositError::HashMismatch {
                reported: hash,
                computed,
            });
        }
        Ok(deposit)
    }

    /// RLP payload length of `[sourceHash, from, to, mint, value, gas, isSystemTx, data]`.
    fn rlp_payload_length(&self) -> usize {
        self.source_hash.length()
            + self.from.length()
            + self.to.length()
            + self.mint.length()
            + self.value.length()
            + self.gas_limit.length()
            + self.is_system_tx.length()
            + self.input.length()
    }

    /// EIP-2718 encoding: `0x7e || rlp([...])`.
    pub fn encode_2718(&self, out: &mut dyn BufMut) {
        out.put_u8(DEPOSIT_TX_TYPE);
        alloy::rlp::Header {
            list: true,
            payload_length: self.rlp_payload_length(),
        }
        .encode(out);
        self.source_hash.encode(out);
        self.from.encode(out);
        self.to.encode(out);
        self.mint.encode(out);
        self.value.encode(out);
        self.gas_limit.encode(out);
        self.is_system_tx.encode(out);
        self.input.encode(out);
    }

    /// Owned EIP-2718 encoding.
    pub fn encoded_2718(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1 + self.rlp_payload_length() + 3);
        self.encode_2718(&mut out);
        out
    }

    /// `to` as an `Option`, `None` for contract creation.
    pub fn to_address(&self) -> Option<Address> {
        match self.to {
            TxKind::Call(addr) => Some(addr),
            TxKind::Create => None,
        }
    }
}

/// Deposit-specific receipt fields, read from the RPC receipt's extra fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DepositReceiptFields {
    /// Sequencer-assigned deposit nonce (Regolith and later).
    pub deposit_nonce: Option<u64>,
    /// Receipt encoding version (Canyon and later, currently always `1`).
    pub deposit_receipt_version: Option<u64>,
}

impl DepositReceiptFields {
    /// Read `depositNonce` / `depositReceiptVersion` from the receipt's catch-all extra fields.
    pub fn from_other_fields(other: &alloy::serde::OtherFields) -> Self {
        let read = |key: &str| {
            other
                .get_deserialized::<U64>(key)
                .and_then(Result::ok)
                .map(|v| v.to::<u64>())
        };
        Self {
            deposit_nonce: read("depositNonce"),
            deposit_receipt_version: read("depositReceiptVersion"),
        }
    }
}

/// Convert an `AnyNetwork` receipt envelope's RPC logs into consensus logs for RLP purposes.
fn consensus_receipt(
    envelope: &AnyReceiptEnvelope<RpcLog>,
) -> ReceiptWithBloom<Receipt<PrimitiveLog>> {
    ReceiptWithBloom {
        receipt: Receipt {
            status: envelope.inner.receipt.status,
            cumulative_gas_used: envelope.inner.receipt.cumulative_gas_used,
            logs: envelope
                .inner
                .receipt
                .logs
                .iter()
                .map(|log| log.inner.clone())
                .collect(),
        },
        logs_bloom: envelope.inner.logs_bloom,
    }
}

/// EIP-2718 encoding of a deposit receipt as used for the header's `receiptsRoot`.
///
/// Mirrors `op-geth`'s `Receipts.EncodeIndex`: when `depositReceiptVersion` is present (Canyon
/// and later) the nonce and version are appended to the standard four receipt fields; otherwise
/// the standard receipt is encoded with the `0x7e` prefix only.
pub fn encode_deposit_receipt_2718(
    envelope: &AnyReceiptEnvelope<RpcLog>,
    fields: DepositReceiptFields,
    tx_hash: B256,
    out: &mut dyn BufMut,
) -> Result<(), DepositError> {
    let receipt = consensus_receipt(envelope);
    let base_len = receipt.receipt.status.length()
        + receipt.receipt.cumulative_gas_used.length()
        + receipt.logs_bloom.length()
        + receipt.receipt.logs.length();

    out.put_u8(DEPOSIT_TX_TYPE);
    match fields.deposit_receipt_version {
        Some(version) => {
            let nonce = fields
                .deposit_nonce
                .ok_or(DepositError::ReceiptMissingNonce { hash: tx_hash })?;
            alloy::rlp::Header {
                list: true,
                payload_length: base_len + nonce.length() + version.length(),
            }
            .encode(out);
            encode_receipt_body(&receipt, out);
            nonce.encode(out);
            version.encode(out);
        }
        None => {
            alloy::rlp::Header {
                list: true,
                payload_length: base_len,
            }
            .encode(out);
            encode_receipt_body(&receipt, out);
        }
    }
    Ok(())
}

fn encode_receipt_body(receipt: &ReceiptWithBloom<Receipt<PrimitiveLog>>, out: &mut dyn BufMut) {
    receipt.receipt.status.encode(out);
    receipt.receipt.cumulative_gas_used.encode(out);
    receipt.logs_bloom.encode(out);
    receipt.receipt.logs.encode(out);
}

/// `txType` value written into the leaf envelope for deposit transactions.
pub const DEPOSIT_LEAF_TX_TYPE: u8 = DEPOSIT_TX_TYPE;

/// ABI-encode the Merkle leaf for a (deposit tx, receipt) pair. See the [module docs](self) for
/// the layout. Returns `None` only if `alloy`'s dynamic ABI encoder refuses a value, which cannot
/// happen for the fixed shapes built here; the `Option` keeps parity with `usc-abi-encoding`.
pub fn abi_encode_deposit_leaf(
    tx: &DepositTransaction,
    rx: &TransactionReceipt<AnyReceiptEnvelope<RpcLog>>,
) -> Option<Vec<u8>> {
    let (is_to_null, to) = match tx.to {
        TxKind::Call(addr) => (false, addr),
        TxKind::Create => (true, Address::ZERO),
    };

    let common = DynSolValue::Tuple(vec![
        DynSolValue::Uint(U256::from(tx.nonce), 64),
        DynSolValue::Uint(U256::from(tx.gas_limit), 64),
        DynSolValue::Address(tx.from),
        DynSolValue::Bool(is_to_null),
        DynSolValue::Address(to),
        DynSolValue::Uint(tx.value, 256),
        DynSolValue::Bytes(tx.input.to_vec()),
    ]);

    let deposit = DynSolValue::Tuple(vec![
        DynSolValue::FixedBytes(tx.source_hash, 32),
        DynSolValue::Uint(tx.mint, 256),
        DynSolValue::Bool(tx.is_system_tx),
    ]);

    let status: u8 = match rx.inner.inner.receipt.status {
        Eip658Value::Eip658(ok) => u8::from(ok),
        // Deposits post-date Byzantium by years; a post-state receipt cannot occur on an
        // OP-Stack chain. Treat as success=1 only if the coerced status says so.
        Eip658Value::PostState(_) => u8::from(rx.inner.inner.receipt.status.coerce_status()),
    };
    let logs = DynSolValue::Array(
        rx.inner
            .inner
            .receipt
            .logs
            .iter()
            .map(|log| {
                DynSolValue::Tuple(vec![
                    DynSolValue::Address(log.address()),
                    DynSolValue::Array(
                        log.topics()
                            .iter()
                            .map(|topic| DynSolValue::FixedBytes(*topic, 32))
                            .collect(),
                    ),
                    DynSolValue::Bytes(log.data().data.to_vec()),
                ])
            })
            .collect(),
    );
    let receipt = DynSolValue::Tuple(vec![
        DynSolValue::Uint(U256::from(status), 8),
        DynSolValue::Uint(U256::from(rx.gas_used), 64),
        logs,
        DynSolValue::Bytes(rx.inner.inner.logs_bloom.0.to_vec()),
    ]);

    let chunks = [common, deposit, receipt]
        .into_iter()
        .map(|chunk| chunk.abi_encode_sequence().map(DynSolValue::Bytes))
        .collect::<Option<Vec<_>>>()?;

    DynSolValue::Tuple(vec![
        DynSolValue::Uint(U256::from(DEPOSIT_LEAF_TX_TYPE), 8),
        DynSolValue::Array(chunks),
    ])
    .abi_encode_sequence()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::dyn_abi::DynSolType;
    use alloy::network::AnyTransactionReceipt;

    /// Base Sepolia block 46388021 (`0x2c3d335`), recorded 2026-09-04 from
    /// `https://base-sepolia-rpc.publicnode.com`. Eleven transactions; index 0 is the
    /// L1-attributes deposit. See `common/eth/tests/fixtures/`.
    const BLOCK_JSON: &str = include_str!("../tests/fixtures/base_sepolia_46388021_block.json");
    const RECEIPTS_JSON: &str =
        include_str!("../tests/fixtures/base_sepolia_46388021_receipts.json");

    fn fixture() -> (alloy::network::AnyRpcBlock, Vec<AnyTransactionReceipt>) {
        (
            serde_json::from_str(BLOCK_JSON).expect("block fixture parses as AnyRpcBlock"),
            serde_json::from_str(RECEIPTS_JSON).expect("receipts fixture parses"),
        )
    }

    #[test]
    fn deposit_parses_and_self_verifies_hash() {
        let (block, _) = fixture();
        let txs: Vec<_> = block
            .inner
            .transactions
            .clone()
            .into_transactions()
            .collect();
        let deposit = DepositTransaction::try_from_rpc(&txs[0]).expect("index 0 is the deposit");
        assert_eq!(
            deposit.from,
            "0xdeaddeaddeaddeaddeaddeaddeaddeaddead0001"
                .parse::<Address>()
                .unwrap()
        );
        assert_eq!(
            deposit.to,
            TxKind::Call(
                "0x4200000000000000000000000000000000000015"
                    .parse::<Address>()
                    .unwrap()
            )
        );
        assert_eq!(deposit.mint, U256::ZERO);
        assert!(!deposit.is_system_tx);
        assert_eq!(deposit.gas_limit, 1_000_000);
        // The hash check inside `try_from_rpc` already passed; make it explicit.
        assert_eq!(keccak256(deposit.encoded_2718()), deposit.hash);
    }

    #[test]
    fn non_deposit_is_rejected_with_its_type() {
        let (block, _) = fixture();
        let txs: Vec<_> = block
            .inner
            .transactions
            .clone()
            .into_transactions()
            .collect();
        let err = DepositTransaction::try_from_rpc(&txs[1]).unwrap_err();
        assert!(
            matches!(err, DepositError::NotADeposit { ty: 2, .. }),
            "{err}"
        );
    }

    #[test]
    fn deposit_receipt_fields_are_read_from_extra_fields() {
        let (_, receipts) = fixture();
        let fields = DepositReceiptFields::from_other_fields(&receipts[0].other);
        assert_eq!(fields.deposit_receipt_version, Some(1));
        assert_eq!(fields.deposit_nonce, Some(46_388_024));
        // A regular tx receipt has neither.
        let none = DepositReceiptFields::from_other_fields(&receipts[1].other);
        assert_eq!(none, DepositReceiptFields::default());
    }

    #[test]
    fn deposit_leaf_has_expected_envelope_and_chunks() {
        let (block, receipts) = fixture();
        let txs: Vec<_> = block
            .inner
            .transactions
            .clone()
            .into_transactions()
            .collect();
        let deposit = DepositTransaction::try_from_rpc(&txs[0]).unwrap();
        let leaf = abi_encode_deposit_leaf(&deposit, &receipts[0].inner).expect("encodes");

        let envelope = DynSolType::Tuple(vec![
            DynSolType::Uint(8),
            DynSolType::Array(Box::new(DynSolType::Bytes)),
        ]);
        let decoded = envelope
            .abi_decode_sequence(&leaf)
            .expect("envelope decodes");
        let DynSolValue::Tuple(parts) = decoded else {
            panic!("expected tuple")
        };
        assert_eq!(parts[0], DynSolValue::Uint(U256::from(126), 8));
        let DynSolValue::Array(chunks) = &parts[1] else {
            panic!("expected chunk array")
        };
        assert_eq!(chunks.len(), 3, "common, deposit, receipt");

        // Common chunk.
        let common_ty = DynSolType::Tuple(vec![
            DynSolType::Uint(64),
            DynSolType::Uint(64),
            DynSolType::Address,
            DynSolType::Bool,
            DynSolType::Address,
            DynSolType::Uint(256),
            DynSolType::Bytes,
        ]);
        let DynSolValue::Bytes(common_bytes) = &chunks[0] else {
            panic!()
        };
        let DynSolValue::Tuple(common) = common_ty.abi_decode_sequence(common_bytes).unwrap()
        else {
            panic!()
        };
        assert_eq!(common[0], DynSolValue::Uint(U256::from(deposit.nonce), 64));
        assert_eq!(common[2], DynSolValue::Address(deposit.from));
        assert_eq!(common[3], DynSolValue::Bool(false));

        // Deposit chunk.
        let deposit_ty = DynSolType::Tuple(vec![
            DynSolType::FixedBytes(32),
            DynSolType::Uint(256),
            DynSolType::Bool,
        ]);
        let DynSolValue::Bytes(dep_bytes) = &chunks[1] else {
            panic!()
        };
        let DynSolValue::Tuple(dep) = deposit_ty.abi_decode_sequence(dep_bytes).unwrap() else {
            panic!()
        };
        assert_eq!(dep[0], DynSolValue::FixedBytes(deposit.source_hash, 32));
        assert_eq!(dep[1], DynSolValue::Uint(U256::ZERO, 256));
        assert_eq!(dep[2], DynSolValue::Bool(false));

        // Receipt chunk: status 1, gasUsed matches receipt.
        let receipt_ty = DynSolType::Tuple(vec![
            DynSolType::Uint(8),
            DynSolType::Uint(64),
            DynSolType::Array(Box::new(DynSolType::Tuple(vec![
                DynSolType::Address,
                DynSolType::Array(Box::new(DynSolType::FixedBytes(32))),
                DynSolType::Bytes,
            ]))),
            DynSolType::Bytes,
        ]);
        let DynSolValue::Bytes(rx_bytes) = &chunks[2] else {
            panic!()
        };
        let DynSolValue::Tuple(rx) = receipt_ty.abi_decode_sequence(rx_bytes).unwrap() else {
            panic!()
        };
        assert_eq!(rx[0], DynSolValue::Uint(U256::from(1), 8));
        assert_eq!(
            rx[1],
            DynSolValue::Uint(U256::from(receipts[0].gas_used), 64)
        );
    }
}
