use crate::clients::usc::{SignedAttestation, SupportedChain};
use anyhow::Result;
use attestor_primitives::{AttestationCheckpoint, AttestationData, BlsSignature};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use parity_scale_codec::{Decode, Encode};
use scale_info::prelude::*;
use sp_core::H256;
use subxt::dynamic::{DecodedValueThunk, Value};
use subxt::ext::scale_value::{Composite, Primitive, ValueDef};
use subxt::utils::AccountId32;
use tracing::{debug, info, warn};

pub fn decode_chain_key_dynamic<T>(val: &Value<T>) -> Option<u64> {
    match &val.value {
        // All numeric primitives come as U128
        ValueDef::Primitive(Primitive::U128(n)) => Some(*n as u64),

        // Tuple-like (unnamed) wrapper: (123)
        ValueDef::Composite(Composite::Unnamed(fields)) if !fields.is_empty() => {
            match &fields[0].value {
                ValueDef::Primitive(Primitive::U128(n)) => Some(*n as u64),
                _ => None,
            }
        }

        // Struct-like (named) wrapper: { some_field: 123 }
        ValueDef::Composite(Composite::Named(fields)) if !fields.is_empty() => {
            let first = &fields[0].1;
            match &first.value {
                ValueDef::Primitive(Primitive::U128(n)) => Some(*n as u64),
                _ => None,
            }
        }

        _ => None,
    }
}

/// Dynamically extract a `SupportedChain` from a generic dynamic Value.
/// Handles named or unnamed composites.
pub fn decode_supported_chain_dynamic<T: std::fmt::Debug>(
    val: &Value<T>,
) -> Option<SupportedChain> {
    let ValueDef::Composite(Composite::Named(fields)) = &val.value else {
        warn!(
            "decode_supported_chain_dynamic: Expected Named Composite, got non-composite value (type {}).",
            std::any::type_name::<T>()
        );
        return None;
    };

    let mut chain_id = None;
    let mut chain_name = None;

    for (name, field) in fields {
        match name.as_str() {
            "chain_id" | "ChainId" => {
                if let ValueDef::Primitive(Primitive::U128(n)) = &field.value {
                    chain_id = Some(*n as u64);
                } else {
                    warn!(
            "decode_supported_chain_dynamic: Unexpected type for 'chain_id' field: {:?}",
            field.value
        );
                }
            }

            "chain_name" | "ChainName" => {
                chain_name = extract_bytes(field);
                if chain_name.is_none() {
                    warn!(
            "decode_supported_chain_dynamic: Failed to extract bytes for 'chain_name'. Got value: {:?}",
            field.value
        );
                }
            }

            other => debug!(
                "decode_supported_chain_dynamic: Ignoring unexpected field '{}'",
                other
            ),
        }
    }

    match (chain_id, chain_name.clone()) {
        (Some(id), Some(name)) => {
            debug!(
                "Successfully decoded SupportedChain: id={}, name={:?}",
                id,
                String::from_utf8_lossy(&name)
            );
            Some(SupportedChain {
                chain_id: id,
                chain_name: name,
            })
        }
        _ => {
            warn!(
                "decode_supported_chain_dynamic: Missing one or more required fields (chain_id={:?}, chain_name={:?})",
                chain_id,
                chain_name.as_ref().map(|v| String::from_utf8_lossy(v))
            );
            None
        }
    }
}

/// Fallback dynamic decoding for AttestationCheckpoint
pub fn decode_checkpoint_dynamic<T: std::fmt::Debug>(
    val: &Value<T>,
) -> Option<AttestationCheckpoint> {
    fn extract_h256<T: std::fmt::Debug>(val: &Value<T>) -> Option<H256> {
        match &val.value {
            // H256 represented directly as a 256-bit integer
            ValueDef::Primitive(Primitive::U256(bytes)) => Some(H256::from(*bytes)),

            // H256 represented as a composite of 32 u8-like integers
            ValueDef::Composite(Composite::Unnamed(inner)) if inner.len() == 32 => {
                let mut arr = [0u8; 32];
                for (i, v) in inner.iter().enumerate().take(32) {
                    match &v.value {
                        ValueDef::Primitive(Primitive::U128(n)) => arr[i] = *n as u8,
                        other => {
                            warn!(
                                "Unexpected value type in H256 composite at index {}: {:?}",
                                i, other
                            );
                            return None; // fail fast — schema mismatch
                        }
                    }
                }
                Some(H256::from(arr))
            }

            _ => None,
        }
    }

    match &val.value {
        // Struct-style (named fields)
        ValueDef::Composite(Composite::Named(fields)) => {
            let mut block_number = None;
            let mut digest = None;

            for (name, field) in fields {
                match name.as_str() {
                    "block_number" | "BlockNumber" => {
                        if let ValueDef::Primitive(Primitive::U128(n)) = &field.value {
                            block_number = Some(*n as u32);
                        }
                    }
                    "digest" | "Digest" => {
                        digest = extract_h256(field);
                    }
                    _ => {}
                }
            }

            block_number
                .zip(digest)
                .map(|(block_number, digest)| AttestationCheckpoint {
                    block_number: block_number.into(),
                    digest,
                })
        }

        // Tuple-style (unnamed fields)
        ValueDef::Composite(Composite::Unnamed(fields)) if fields.len() >= 2 => {
            let block_number = match &fields[0].value {
                ValueDef::Primitive(Primitive::U128(n)) => Some(*n as u32),
                _ => None,
            };
            let digest = extract_h256(&fields[1]);

            block_number
                .zip(digest)
                .map(|(block_number, digest)| AttestationCheckpoint {
                    block_number: block_number.into(),
                    digest,
                })
        }

        _ => None,
    }
}

/// Generic helper: decode statically, or use dynamic fallback via closure.
///
/// - `thunk` is the dynamic storage value (`DecodedValueThunk`).
/// - `fallback` is a closure that knows how to extract the struct from `Value<T>`.
/// - Returns `Some(T)` or `None` if missing.
/// - Generic helper: try static decode first, fallback to dynamic.
pub fn decode_static_or_dynamic<T>(
    maybe_val: &Option<DecodedValueThunk>,
    fallback: impl FnOnce(&DecodedValueThunk) -> Option<T>,
) -> Result<Option<T>>
where
    T: Decode,
{
    if let Some(thunk) = maybe_val {
        // 1. Try static SCALE decode first
        let bytes = thunk.encoded();
        let mut input = bytes;
        let initial_len = input.len();

        match T::decode(&mut input) {
            Ok(decoded) => {
                let remaining = input.len();
                let consumed = initial_len - remaining;

                if remaining > 0 {
                    info!(
                        "⚠️ Static decode for `{}` succeeded but left {} unread bytes ({} consumed) — possible runtime schema drift.",
                        std::any::type_name::<T>(),
                        remaining,
                        consumed
                    );
                } else {
                    debug!(
                        "✅ Static decode succeeded cleanly for `{}` ({} bytes consumed)",
                        std::any::type_name::<T>(),
                        consumed
                    );
                }

                return Ok(Some(decoded));
            }

            Err(err) => {
                warn!(
                    "❌ Static decode failed for `{}`: {:?}. Attempting dynamic fallback...",
                    std::any::type_name::<T>(),
                    err
                );
            }
        }

        // 2. Fallback to dynamic decode
        match fallback(thunk) {
            Some(dynamic_decoded) => {
                info!(
                    "🌀 Dynamic fallback **succeeded** for `{}` — static decode failed earlier. \
                     This usually indicates a runtime schema change.",
                    std::any::type_name::<T>()
                );
                Ok(Some(dynamic_decoded))
            }
            None => {
                info!(
                    "💥 Both static and dynamic decode failed for `{}` — returning None.",
                    std::any::type_name::<T>()
                );
                Ok(None)
            }
        }
    } else {
        info!(
            "decode_static_or_dynamic: no value present for `{}` — returning None.",
            std::any::type_name::<T>()
        );
        Ok(None)
    }
}
/// Try to decode a `SignedAttestation` by falling back to a composite.
pub fn decode_signed_attestation_dynamic<T>(
    val: &Value<T>,
) -> Option<SignedAttestation<H256, AccountId32>> {
    let ValueDef::Composite(Composite::Named(fields)) = &val.value else {
        warn!(
            "decode_signed_attestation_dynamic: Expected Named Composite, got non-composite value (type {}).",
            std::any::type_name::<T>()
        );
        return None;
    };

    let mut attestation_bytes = None;
    let mut signature_bytes = None;
    let mut attestors_bytes = None;

    for (name, field) in fields {
        let bytes = match value_to_scale_bytes(field) {
            Some(b) => b,
            None => {
                info!(
                    "decode_signed_attestation_dynamic: Failed to extract bytes for field '{}'",
                    name
                );
                continue;
            }
        };

        match name.as_str() {
            "attestation" => attestation_bytes = Some(bytes),
            "signature" => signature_bytes = Some(bytes),
            "attestors" => attestors_bytes = Some(bytes),
            _ => debug!(
                "decode_signed_attestation_dynamic: Ignoring unexpected field '{}'",
                name
            ),
        }
    }

    if let (Some(att_bytes), Some(sig_bytes), Some(att_vec_bytes)) =
        (attestation_bytes, signature_bytes, attestors_bytes)
    {
        let attestation = match AttestationData::<H256>::decode(&mut &att_bytes[..]) {
            Ok(a) => a,
            Err(e) => {
                warn!("Failed to decode Attestation: {:?}", e);
                return None;
            }
        };

        let signature = match BlsSignature::decode(&mut &sig_bytes[..]) {
            Ok(s) => s,
            Err(e) => {
                warn!("Failed to decode BlsSignature: {:?}", e);
                return None;
            }
        };

        let attestors = match Vec::<AccountId32>::decode(&mut &att_vec_bytes[..]) {
            Ok(v) => v,
            Err(e) => {
                warn!("Failed to decode attestors Vec<AccountId32>: {:?}", e);
                return None;
            }
        };

        debug!(
            "Successfully decoded SignedAttestation ({} attestors)",
            attestors.len()
        );

        Some(SignedAttestation {
            attestation,
            signature,
            attestors,
        })
    } else {
        warn!("decode_signed_attestation_dynamic: Missing one or more required fields");
        None
    }
}

pub fn decode_interval_dynamic<T>(val: &Value<T>) -> Option<u32> {
    match &val.value {
        ValueDef::Primitive(Primitive::U128(n)) => Some(*n as u32),

        ValueDef::Composite(Composite::Unnamed(fields)) if !fields.is_empty() => {
            match &fields[0].value {
                ValueDef::Primitive(Primitive::U128(n)) => Some(*n as u32),
                _ => None,
            }
        }

        ValueDef::Composite(Composite::Named(fields)) if !fields.is_empty() => {
            let first = &fields[0].1;
            match &first.value {
                ValueDef::Primitive(Primitive::U128(n)) => Some(*n as u32),
                _ => None,
            }
        }

        _ => None,
    }
}

fn extract_bytes<T>(val: &Value<T>) -> Option<Vec<u8>> {
    match &val.value {
        // Vec<u8> represented as unnamed tuple (e.g. [u8; N])
        ValueDef::Composite(Composite::Unnamed(elements)) => {
            let mut out = Vec::with_capacity(elements.len());
            for el in elements {
                if let ValueDef::Primitive(Primitive::U128(n)) = &el.value {
                    out.push(*n as u8);
                }
            }
            Some(out)
        }

        // BitSequence fallback: use the Encode impl
        ValueDef::BitSequence(bits) => Some(bits.encode()),

        _ => None,
    }
}

/// Recursively convert a dynamic `Value` into raw SCALE-like bytes.
///
/// Used to reconstruct encoded data for dynamic decoding (e.g. `SignedAttestation`).
/// This function handles all relevant `ValueDef` variants including `Variant` and `BitSequence`.
pub fn value_to_scale_bytes<T>(val: &Value<T>) -> Option<Vec<u8>> {
    match &val.value {
        ValueDef::Primitive(p) => match p {
            Primitive::Bool(b) => Some(vec![*b as u8]),
            Primitive::Char(c) => Some(vec![*c as u8]),
            Primitive::String(s) => BASE64.decode(s).ok(),
            Primitive::U128(n) => Some(n.to_le_bytes().to_vec()),
            Primitive::I128(n) => Some(n.to_le_bytes().to_vec()),
            Primitive::U256(arr) => Some(arr.to_vec()),
            Primitive::I256(arr) => Some(arr.to_vec()),
        },

        ValueDef::Composite(Composite::Unnamed(fields)) => {
            let mut out = Vec::new();
            for f in fields {
                out.extend(value_to_scale_bytes(f)?);
            }
            Some(out)
        }

        ValueDef::Composite(Composite::Named(fields)) => {
            let mut out = Vec::new();
            for (_name, f) in fields {
                out.extend(value_to_scale_bytes(f)?);
            }
            Some(out)
        }

        ValueDef::Variant(v) => {
            let mut out = Vec::new();
            out.extend(v.name.as_bytes());
            match &v.values {
                Composite::Unnamed(fields) => {
                    for f in fields {
                        out.extend(value_to_scale_bytes(f)?);
                    }
                }
                Composite::Named(fields) => {
                    for (_n, f) in fields {
                        out.extend(value_to_scale_bytes(f)?);
                    }
                }
            }
            Some(out)
        }

        ValueDef::BitSequence(bits) => Some(bits.encode()),
    }
}
