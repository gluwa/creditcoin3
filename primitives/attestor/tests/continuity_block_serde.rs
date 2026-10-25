use attestor_primitives::ContinuityBlock;
use serde_json::Value;
use sp_core::H256;

fn h256_from_u64(n: u64) -> H256 {
    let mut bytes = [0u8; 32];
    bytes[24..32].copy_from_slice(&n.to_be_bytes());
    H256::from(bytes)
}

#[test]
fn continuity_block_json_format_and_roundtrip() {
    let block = ContinuityBlock {
        merkle_root: h256_from_u64(123),
        digest: h256_from_u64(456),
    };
    let json = serde_json::to_string(&block).expect("serialize");
    let v: Value = serde_json::from_str(&json).expect("parse json");

    for field in ["merkle_root", "digest"] {
        let s = v
            .get(field)
            .expect("field exists")
            .as_str()
            .expect("string");
        assert!(s.starts_with("0x"), "{field} must have 0x prefix");
        assert_eq!(s.len(), 66, "{field} must be 0x + 64 hex chars");
    }

    let rt: ContinuityBlock = serde_json::from_str(&json).expect("round trip");
    assert_eq!(rt.merkle_root, block.merkle_root);
    assert_eq!(rt.digest, block.digest);
}
