//! OP-Stack specifics: parsing and root-checking `0x7e` **deposit transactions** and receipts.
//!
//! OP-Stack rollups (Base, OP Mainnet, …) are Ethereum-equivalent except for one extra
//! transaction type. Every L2 block opens with a deposit transaction that writes the L1 block
//! attributes into the `L1Block` predeploy, and user deposits through the `OptimismPortal` are
//! delivered the same way. Deposit transactions are *unsigned* — they are derived from L1 — and
//! carry three fields no L1 type has: `sourceHash`, `mint` and `isSystemTx`.
//!
//! alloy's `Ethereum` network types reject the `0x7e` type outright, so an attestor typed on them
//! cannot even parse a Base block. The deposit transaction type, its EIP-2718 encoding and the
//! Merkle leaf layout (`txType = 126`) live in `attestcoin-abi-encoding`
//! ([`attestcoin_abi_encoding::abi::op_stack`]) and are re-exported here. This module adds the
//! attestor side:
//!
//! - [`deposit_from_rpc`] / [`deposit_from_unknown`]: parse a [`DepositTransaction`] from the
//!   `AnyNetwork` "unknown transaction" catch-all. The EIP-2718 encoding is self-checked against
//!   the RPC-reported hash, so a wrong field layout can never silently produce a bogus root.
//! - [`receipt_fields_from_other`]: read `depositNonce` / `depositReceiptVersion` from the RPC
//!   receipt.
//! - [`encode_deposit_receipt_2718`]: the receipt encoding used for `receiptsRoot`, including the
//!   post-Canyon `depositNonce` / `depositReceiptVersion` tail.
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
//! # Nonce and gas canonicalization
//!
//! `nonce` is the receipt's `depositNonce` when `depositReceiptVersion` is present (Canyon
//! and later), so it is authenticated by `receiptsRoot`. Before Canyon neither the transaction
//! nor receipt root commits to a nonce: the leaf uses **zero**, meaning unavailable, even if
//! RPC metadata reports a nonzero execution nonce.
//!
//! `gasUsed` is the difference between this receipt's authenticated `cumulativeGasUsed` and
//! the preceding receipt's value (zero before the first transaction). The block pipeline
//! canonicalizes it before leaf encoding; the RPC `gasUsed` field is unauthenticated metadata.

use alloy::{
    consensus::{Receipt, ReceiptWithBloom},
    network::{AnyReceiptEnvelope, AnyRpcTransaction, AnyTxEnvelope, UnknownTxEnvelope},
    primitives::{Address, Bytes, Log as PrimitiveLog, TxKind, B256, U256, U64},
    rlp::{BufMut, Encodable},
    rpc::types::Log as RpcLog,
};

pub use attestcoin_abi_encoding::abi::op_stack::{
    DepositReceiptFields, DepositTransaction, DEPOSIT_TX_TYPE,
};

/// Errors turning RPC payloads into [`DepositTransaction`]s.
#[derive(Debug, thiserror::Error)]
pub enum DepositError {
    /// The transaction is not a `0x7e` deposit.
    #[error("transaction {hash} has type {ty:#x}, not a deposit (0x7e)")]
    NotADeposit { hash: B256, ty: u8 },
    /// A type `0x7e` transaction without a `sourceHash`. Every OP-Stack deposit carries one, so
    /// the source is not an OP-Stack chain: Scroll, for one, reuses type `0x7e` for its L1
    /// messages with a different layout. Another endpoint of the same source cannot serve it.
    #[error(
        "transaction {hash} has type 0x7e but no `sourceHash`, so it is not an OP-Stack deposit \
         (the source is probably not an OP-Stack chain; Scroll L1 messages also use type 0x7e)"
    )]
    NotOpStack { hash: B256 },
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
    #[error("deposit receipt for {hash}: malformed field `{field}`")]
    ReceiptField { hash: B256, field: &'static str },
    #[error("deposit transaction {hash}: RPC nonce {transaction_nonce} differs from receipt nonce {receipt_nonce}")]
    NonceMismatch {
        hash: B256,
        transaction_nonce: u64,
        receipt_nonce: u64,
    },
    #[error("deposit receipt for {hash}: cumulative gas {cumulative} is less than the previous receipt's {previous}")]
    DecreasingCumulativeGas {
        hash: B256,
        previous: u64,
        cumulative: u64,
    },
}

impl DepositError {
    /// The payload shows the source chain is not in the configured family at all, which no
    /// other endpoint of the same source can fix: a misconfiguration, not a flaky peer.
    pub fn is_wrong_family(&self) -> bool {
        matches!(self, Self::NotOpStack { .. })
    }
}

/// Parse a deposit from an `AnyNetwork` RPC transaction and verify its hash.
///
/// Fails with [`DepositError::NotADeposit`] if the transaction is anything else, so callers
/// can match on the type byte first without a second inspection.
pub fn deposit_from_rpc(
    tx: &AnyRpcTransaction,
    receipt: DepositReceiptFields,
) -> Result<DepositTransaction, DepositError> {
    let unknown = match tx.inner.inner.inner() {
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
    deposit_from_unknown(unknown, tx.inner.inner.signer(), receipt)
}

/// Parse a deposit from the catch-all envelope plus the RPC-level `from`.
pub fn deposit_from_unknown(
    unknown: &UnknownTxEnvelope,
    from: Address,
    receipt: DepositReceiptFields,
) -> Result<DepositTransaction, DepositError> {
    let hash = unknown.hash;
    if unknown.inner.ty.0 != DEPOSIT_TX_TYPE {
        return Err(DepositError::NotADeposit {
            hash,
            ty: unknown.inner.ty.0,
        });
    }
    let fields = &unknown.inner.fields;
    let field = |name: &'static str| DepositError::Field { hash, field: name };

    let source_hash: B256 = match fields.get_deserialized::<B256>("sourceHash") {
        Some(Ok(v)) => v,
        Some(Err(_)) => return Err(field("sourceHash")),
        None => return Err(DepositError::NotOpStack { hash }),
    };
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
    // RPC transaction nonces are metadata, absent on some historical providers. Only
    // Canyon receipts authenticate this field. Before Canyon use zero regardless of
    // either RPC object's metadata so all providers produce the same leaf.
    let nonce = canonical_nonce(receipt, hash)?;
    if receipt.deposit_receipt_version.is_some() {
        let rpc_nonce = fields
            .get_deserialized::<Option<U64>>("nonce")
            .transpose()
            .map_err(|_| field("nonce"))?
            .flatten();
        if let Some(rpc_nonce) = rpc_nonce {
            let transaction_nonce = rpc_nonce.to();
            if transaction_nonce != nonce {
                return Err(DepositError::NonceMismatch {
                    hash,
                    transaction_nonce,
                    receipt_nonce: nonce,
                });
            }
        }
    }

    let deposit = DepositTransaction {
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
    let computed = deposit.computed_hash();
    if computed != hash {
        return Err(DepositError::HashMismatch {
            reported: hash,
            computed,
        });
    }
    Ok(deposit)
}

/// Read `depositNonce` / `depositReceiptVersion` from the receipt's catch-all extra fields.
pub fn receipt_fields_from_other(
    other: &alloy::serde::OtherFields,
    hash: B256,
) -> Result<DepositReceiptFields, DepositError> {
    let read = |key: &'static str| {
        other
            .get_deserialized::<Option<U64>>(key)
            .transpose()
            .map_err(|_| DepositError::ReceiptField { hash, field: key })
            .map(|value| value.flatten().map(|v| v.to::<u64>()))
    };
    let fields = DepositReceiptFields {
        deposit_nonce: read("depositNonce")?,
        deposit_receipt_version: read("depositReceiptVersion")?,
    };
    canonical_nonce(fields, hash)?;
    Ok(fields)
}

/// The nonce that goes into the leaf: the receipt's from Canyon on, zero before. Wraps the SDK's
/// rule so a missing nonce reports the offending transaction hash.
fn canonical_nonce(fields: DepositReceiptFields, hash: B256) -> Result<u64, DepositError> {
    fields
        .canonical_nonce()
        .map_err(|_| DepositError::ReceiptMissingNonce { hash })
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

#[cfg(test)]
mod tests {
    use super::*;
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
        let (block, receipts) = fixture();
        let txs: Vec<_> = block
            .inner
            .transactions
            .clone()
            .into_transactions()
            .collect();
        let deposit = deposit_from_rpc(
            &txs[0],
            receipt_fields_from_other(&receipts[0].other, receipts[0].transaction_hash).unwrap(),
        )
        .expect("index 0 is the deposit");
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
        // The hash check inside `deposit_from_rpc` already passed; make it explicit.
        assert_eq!(deposit.computed_hash(), deposit.hash);
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
        let err = deposit_from_rpc(&txs[1], DepositReceiptFields::default()).unwrap_err();
        assert!(
            matches!(err, DepositError::NotADeposit { ty: 2, .. }),
            "{err}"
        );
    }

    #[test]
    fn deposit_receipt_fields_are_read_from_extra_fields() {
        let (_, receipts) = fixture();
        let fields =
            receipt_fields_from_other(&receipts[0].other, receipts[0].transaction_hash).unwrap();
        assert_eq!(fields.deposit_receipt_version, Some(1));
        assert_eq!(fields.deposit_nonce, Some(46_388_024));
        // A regular tx receipt has neither.
        let none =
            receipt_fields_from_other(&receipts[1].other, receipts[1].transaction_hash).unwrap();
        assert_eq!(none, DepositReceiptFields::default());
    }
}
