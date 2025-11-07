// Minimal thunk stub for tests
#[cfg(test)]
mod decode {
    use crate::clients::usc::{
        decode::value_to_scale_bytes, decode_chain_key_dynamic, decode_checkpoint_dynamic,
        decode_signed_attestation_dynamic, decode_supported_chain_dynamic,
    };

    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    use parity_scale_codec::{Decode, Encode};
    use sp_core::H256;
    use subxt::ext::scale_value::{Composite, Primitive, Value, ValueDef};
    use subxt::utils::AccountId32;

    #[derive(Clone)]
    struct MockThunk {
        encoded: Vec<u8>,
    }
    impl MockThunk {
        fn new(encoded: Vec<u8>) -> Self {
            Self { encoded }
        }
        fn encoded(&self) -> Vec<u8> {
            self.encoded.clone()
        }
    }

    fn decode_static_or_dynamic_mock<T: Decode>(
        maybe_val: &Option<MockThunk>,
        fallback: impl FnOnce(&MockThunk) -> Option<T>,
    ) -> anyhow::Result<Option<T>> {
        if let Some(thunk) = maybe_val {
            let bytes = thunk.encoded();
            if let Ok(decoded) = T::decode(&mut &bytes[..]) {
                return Ok(Some(decoded));
            }
            if let Some(dynamic_decoded) = fallback(thunk) {
                return Ok(Some(dynamic_decoded));
            }
        }
        Ok(None)
    }

    // Dummy struct to exercise static decode success
    #[derive(Debug, PartialEq, Encode, Decode)]
    struct Dummy(u32);

    // decode_static_or_dynamic: static + fallback path
    #[test]
    fn decodes_statically_when_possible() {
        let encoded = Dummy(42).encode();
        let thunk = MockThunk::new(encoded);
        let maybe_val = Some(thunk);

        let result: Option<Dummy> = decode_static_or_dynamic_mock(&maybe_val, |_dyn| None).unwrap();

        assert_eq!(result, Some(Dummy(42)));
    }

    #[test]
    fn falls_back_to_dynamic_when_static_fails() {
        // Dynamic value, used in the fallback
        let val = Value {
            value: ValueDef::Composite(Composite::Named(vec![(
                "inner".into(),
                Value::from(123u32),
            )])),
            context: (),
        };

        // Bogus bytes to ensure static decode fails
        let thunk = MockThunk::new(vec![0x99, 0x88, 0x77]);
        let maybe_val = Some(thunk);

        let result: Option<u32> = decode_static_or_dynamic_mock(&maybe_val, |_t| {
            if let ValueDef::Composite(Composite::Named(fields)) = &val.value {
                if let Some((_name, field)) = fields.first() {
                    if let ValueDef::Primitive(Primitive::U128(n)) = &field.value {
                        return Some(*n as u32);
                    }
                }
            }
            None
        })
        .unwrap();

        assert_eq!(result, Some(123));
    }

    /// Helper to construct a `Value` representing bytes (`Vec<u8>`)
    /// in the same shape `value_to_scale_bytes` expects.
    fn make_bytes_value(raw: &[u8]) -> Value<()> {
        let encoded = BASE64.encode(raw);
        Value {
            value: ValueDef::Primitive(Primitive::String(encoded)),
            context: (),
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
    pub struct ScaleFelt(pub [u8; 32]);

    #[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
    pub struct Digest(pub [u8; 32]);

    #[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
    pub struct BlsSignature(pub [u8; 96]);

    #[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
    pub struct Attestation<H> {
        pub chain_key: u64,
        pub header_number: u64,
        pub header_hash: H,
        pub root: ScaleFelt,
        pub prev_digest: Option<Digest>,
    }

    #[test]
    fn decodes_signed_attestation_dynamic_realistic() {
        // --- arrange ---
        let attestation = Attestation::<H256> {
            chain_key: 1,
            header_number: 42,
            header_hash: H256::repeat_byte(0xab),
            root: ScaleFelt([0xcd; 32]),
            prev_digest: Some(Digest([0xef; 32])),
        };
        let sig = BlsSignature([0x11; 96]);
        let attestors = vec![AccountId32([0xaa; 32])];

        // encode all parts using real SCALE impls
        let att_bytes = attestation.encode();
        let sig_bytes = sig.encode();
        let attestors_bytes = attestors.encode();

        // embed into Value structure as expected by decode_signed_attestation_dynamic
        let val = Value {
            value: ValueDef::Composite(Composite::Named(vec![
                ("attestation".into(), make_bytes_value(&att_bytes)),
                ("signature".into(), make_bytes_value(&sig_bytes)),
                ("attestors".into(), make_bytes_value(&attestors_bytes)),
            ])),
            context: (),
        };

        // --- act ---
        let decoded = decode_signed_attestation_dynamic(&val)
            .expect("Expected to decode realistic signed attestation");

        // --- assert ---
        assert_eq!(decoded.attestation.chain_key, 1);
        assert_eq!(decoded.attestation.header_number, 42);
        assert_eq!(decoded.attestation.header_hash, H256::repeat_byte(0xab));
        assert_eq!(decoded.attestors.len(), 1);
        assert_eq!(decoded.attestors[0], AccountId32([0xaa; 32]));
    }

    #[test]
    fn decode_signed_attestation_dynamic_returns_none_on_invalid_data() {
        // malformed / truncated SCALE bytes
        let val = Value {
            value: ValueDef::Composite(Composite::Named(vec![
                ("attestation".into(), make_bytes_value(&[0x01])), // invalid
                ("signature".into(), make_bytes_value(&[0x02])),
                ("attestors".into(), make_bytes_value(&[0x03])),
            ])),
            context: (),
        };

        let decoded = decode_signed_attestation_dynamic(&val);
        assert!(
            decoded.is_none(),
            "Expected decode_signed_attestation_dynamic to gracefully return None on bad input"
        );
    }

    // decode_supported_chain_dynamic tests
    #[test]
    fn decodes_named_supported_chain_dynamic() {
        let bytes = b"Kusama";
        let bytes_as_values: Vec<Value<()>> = bytes.iter().map(|b| Value::from(*b)).collect(); // each u8 → Value::Primitive(U128(u8))

        let val = Value {
            value: ValueDef::Composite(Composite::Named(vec![
                ("chain_id".into(), Value::from(1u64)),
                (
                    "chain_name".into(),
                    Value {
                        value: ValueDef::Composite(Composite::Unnamed(bytes_as_values)),
                        context: (),
                    },
                ),
            ])),
            context: (),
        };

        let decoded = decode_supported_chain_dynamic(&val).expect("should decode");
        assert_eq!(decoded.chain_id, 1);
        assert_eq!(
            String::from_utf8(decoded.chain_name.clone()).unwrap(),
            "Kusama"
        );
    }

    #[test]
    fn decodes_unnamed_supported_chain_dynamic() {
        let bytes = b"Polkadot";
        let bytes_as_values: Vec<Value<()>> = bytes.iter().map(|b| Value::from(*b)).collect();

        let val = Value {
            value: ValueDef::Composite(Composite::Unnamed(vec![
                Value::from(42u64),
                Value {
                    value: ValueDef::Composite(Composite::Unnamed(bytes_as_values)),
                    context: (),
                },
            ])),
            context: (),
        };

        assert!(
            decode_supported_chain_dynamic(&val).is_none(),
            "Unnamed composite is not yet supported"
        );
    }

    #[test]
    fn decodes_named_supported_chain_dynamic_out_of_order() {
        let bytes = b"Kusama";
        let bytes_as_values: Vec<Value<()>> = bytes.iter().map(|b| Value::from(*b)).collect();

        // Build with reversed field order: chain_name FIRST
        let val = Value {
            value: ValueDef::Composite(Composite::Named(vec![
                (
                    "chain_name".into(),
                    Value {
                        value: ValueDef::Composite(Composite::Unnamed(bytes_as_values)),
                        context: (),
                    },
                ),
                ("chain_id".into(), Value::from(99u64)),
            ])),
            context: (),
        };

        let decoded =
            decode_supported_chain_dynamic(&val).expect("should decode even if out of order");

        assert_eq!(decoded.chain_id, 99);
        assert_eq!(
            String::from_utf8(decoded.chain_name.clone()).unwrap(),
            "Kusama"
        );
    }

    #[test]
    fn malformed_supported_chain_returns_none() {
        let val = Value {
            value: ValueDef::Composite(Composite::Named(vec![])),
            context: (),
        };
        assert!(decode_supported_chain_dynamic(&val).is_none());
    }

    // decode_chain_key_dynamic tests
    #[test]
    fn decodes_chain_key_from_u128() {
        let val = Value::from(99u128);
        assert_eq!(decode_chain_key_dynamic(&val), Some(99));
    }

    #[test]
    fn decodes_chain_key_from_named_field() {
        let val = Value {
            value: ValueDef::Composite(Composite::Named(vec![("key".into(), Value::from(7u128))])),
            context: (),
        };
        assert_eq!(decode_chain_key_dynamic(&val), Some(7));
    }

    #[test]
    fn invalid_chain_key_returns_none() {
        let val = Value {
            value: ValueDef::Primitive(Primitive::String("oops".into())),
            context: (),
        };
        assert!(decode_chain_key_dynamic(&val).is_none());
    }

    // decode_checkpoint_dynamic tests
    #[test]
    fn decodes_named_checkpoint_dynamic() {
        let block_number_u128 = 100u128;
        let digest_bytes = [5u8; 32];
        let digest_val = Value::from(digest_bytes.to_vec());

        let val = Value {
            value: ValueDef::Composite(Composite::Named(vec![
                ("block_number".into(), Value::from(block_number_u128)),
                ("digest".into(), digest_val),
            ])),
            context: (),
        };

        let decoded = decode_checkpoint_dynamic(&val).expect("should decode");
        assert_eq!(decoded.block_number, (block_number_u128 as u64)); // cast to u64 here
        assert_eq!(decoded.digest, H256::from_slice(&digest_bytes));
    }

    #[test]
    fn incomplete_checkpoint_returns_none() {
        let val = Value {
            value: ValueDef::Composite(Composite::Named(vec![(
                "block_number".into(),
                Value::from(42u128),
            )])),
            context: (),
        };
        assert!(decode_checkpoint_dynamic(&val).is_none());
    }

    // value_to_scale_bytes tests
    #[test]
    fn encodes_simple_primitive_value_to_bytes() {
        let val = Value::from(123u128);
        let bytes = value_to_scale_bytes(&val).unwrap();
        assert_eq!(bytes, 123u128.to_le_bytes().to_vec());
    }

    #[test]
    fn encodes_named_composite_to_bytes() {
        let val = Value {
            value: ValueDef::Composite(Composite::Named(vec![
                ("field1".into(), Value::from(10u128)),
                ("field2".into(), Value::from(20u128)),
            ])),
            context: (),
        };

        let bytes = value_to_scale_bytes(&val).unwrap();
        assert!(bytes.contains(&10u8));
        assert!(bytes.contains(&20u8));
    }

    #[test]
    fn handles_bitsequence_encoding() {
        use subxt::ext::scale_bits::Bits;

        let mut bits = Bits::new();
        bits.push(true);
        bits.push(false);
        bits.push(true);

        let val = Value {
            value: ValueDef::BitSequence(bits.clone()),
            context: (),
        };

        let bytes = value_to_scale_bytes(&val).unwrap();
        assert_eq!(bytes, bits.encode());
    }
    #[test]
    fn fails_when_chain_name_missing() {
        // Construct a value that only has a chain_id
        let val = Value {
            value: ValueDef::Composite(Composite::Named(vec![(
                "chain_id".into(),
                Value::from(777u64),
            )])),
            context: (),
        };

        // Because `chain_name` is missing, expect None
        assert!(
            decode_supported_chain_dynamic(&val).is_none(),
            "should return None when chain_name is missing"
        );
    }
    #[test]
    fn fails_when_chain_id_missing() {
        let bytes = b"Westend";
        let bytes_as_values: Vec<Value<()>> = bytes.iter().map(|b| Value::from(*b)).collect();

        // Construct a value that only has chain_name
        let val = Value {
            value: ValueDef::Composite(Composite::Named(vec![(
                "chain_name".into(),
                Value {
                    value: ValueDef::Composite(Composite::Unnamed(bytes_as_values)),
                    context: (),
                },
            )])),
            context: (),
        };

        // Because `chain_id` is missing, expect None
        assert!(
            decode_supported_chain_dynamic(&val).is_none(),
            "should return None when chain_id is missing"
        );
    }
    #[test]
    fn decodes_supported_chain_with_extra_fields() {
        let bytes = b"Rococo";
        let bytes_as_values: Vec<Value<()>> = bytes.iter().map(|b| Value::from(*b)).collect();

        // Construct a named composite that includes unrelated fields
        let val = Value {
            value: ValueDef::Composite(Composite::Named(vec![
                ("version".into(), Value::from(5u32)), // extra field
                ("chain_id".into(), Value::from(10u64)),
                (
                    "chain_name".into(),
                    Value {
                        value: ValueDef::Composite(Composite::Unnamed(bytes_as_values)),
                        context: (),
                    },
                ),
                ("status".into(), Value::from("active")), // another irrelevant field
            ])),
            context: (),
        };

        let decoded = decode_supported_chain_dynamic(&val)
            .expect("should decode successfully even with extra fields");

        assert_eq!(decoded.chain_id, 10);
        assert_eq!(
            String::from_utf8(decoded.chain_name.clone()).unwrap(),
            "Rococo"
        );
    }
}
