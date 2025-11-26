use mmr::query_proof::{MerkleProofEntry, QueryMerkleProof};
use serde_json::Value;
use sp_core::H256;

// Ensure serialization format: 0x-prefixed lowercase hex, 64 nybbles, length 66.
#[test]
fn query_merkle_proof_serialization_format() {
    let root = H256::from([0x11u8; 32]);
    let siblings = vec![
        MerkleProofEntry {
            hash: H256::from([0x22u8; 32]),
            is_left: true,
        },
        MerkleProofEntry {
            hash: H256::from([0x33u8; 32]),
            is_left: false,
        },
    ];
    let proof = QueryMerkleProof::new(root, siblings.clone());

    let json = serde_json::to_string(&proof).expect("serialize proof");
    let v: Value = serde_json::from_str(&json).expect("json parse");

    // root format checks
    let root_str = v
        .get("root")
        .expect("root field")
        .as_str()
        .expect("root string");
    assert!(
        root_str.starts_with("0x"),
        "root must be 0x-prefixed: {root_str}"
    );
    assert_eq!(
        root_str.len(),
        66,
        "root hex length must be 66 chars including 0x"
    );
    assert!(
        root_str
            .chars()
            .skip(2)
            .all(|c| c.is_ascii_hexdigit() && c.is_ascii_lowercase() || c.is_ascii_digit()),
        "root must be lowercase hex"
    );

    // sibling hash format checks
    let siblings_v = v
        .get("siblings")
        .expect("siblings field")
        .as_array()
        .expect("siblings array");
    assert_eq!(siblings_v.len(), siblings.len());
    for (i, sib) in siblings_v.iter().enumerate() {
        let hash_str = sib
            .get("hash")
            .expect("hash field")
            .as_str()
            .expect("hash string");
        assert!(
            hash_str.starts_with("0x"),
            "sibling[{i}] hash must be 0x-prefixed"
        );
        assert_eq!(hash_str.len(), 66, "sibling[{i}] hash length must be 66");
    }

    // round-trip
    let decoded: QueryMerkleProof = serde_json::from_str(&json).expect("round trip deserialize");
    assert_eq!(decoded, proof);
}
