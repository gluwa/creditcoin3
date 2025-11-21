use serde_json::json;

pub fn get_mock_continuity_proof(chain_key: u64, header_number: u64) -> serde_json::Value {
    json!({
        "chain_key": chain_key,
        "header_number": header_number,
        "continuity_proof": {
            "lower_endpoint_digest": "0x1111",
            "blocks": [
                { "root": "0xaaaa", "digest": "0xbbbb" },
                { "root": "0xcccc", "digest": "0xdddd" }
            ]
        },
        "cached": false,
        "generated_at": "2025-01-01T00:00:00Z"
    })
}

pub fn get_mock_continuity_proof_with_tx(
    chain_key: u64,
    header_number: u64,
    tx_index: u64,
) -> serde_json::Value {
    json!({
        "chain_key": chain_key,
        "header_number": header_number,
        "tx_index": tx_index,
        "continuity_proof": {
            "lower_endpoint_digest": "0x1111",
            "blocks": [
                { "root": "0xaaaa", "digest": "0xbbbb" },
                { "root": "0xcccc", "digest": "0xdddd" }
            ]
        },
        "merkle_proof": {
            "root": "0xeeee",
            "siblings": [
                { "hash": "0xffff", "is_left": false }
            ]
        },
        "merkle_root": "0xeeee",
        "cached": false,
        "generated_at": "2025-01-01T00:00:00Z"
    })
}

pub fn get_mock_proof_by_tx_hash(chain_key: u64, tx_hash: String) -> serde_json::Value {
    json!({
        "chain_key": chain_key,
        "tx_hash": tx_hash,
        "header_number": 12345,
        "tx_index": 0,
        "continuity_proof": {
            "lower_endpoint_digest": "0x9999",
            "blocks": [
                { "root": "0xaaaa", "digest": "0xbbbb" },
                { "root": "0xcccc", "digest": "0xdddd" }
            ]
        },
        "merkle_proof": {
            "root": "0xeeee",
            "siblings": [
                { "hash": "0xffff", "is_left": false }
            ]
        },
        "merkle_root": "0xeeee",
        "cached": false,
        "generated_at": "2025-01-01T00:00:00Z"
    })
}
