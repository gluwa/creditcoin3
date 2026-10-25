//! Tests for the indexer client

use crate::{types::QueryVariables, IndexerClient};
use sp_core::H256;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn test_query_variables_serialization() {
    let vars = QueryVariables {
        chain_key: "1".to_string(),
        header_number: "100".to_string(),
    };
    let json = serde_json::to_string(&vars).unwrap();
    assert!(json.contains("chainKey"));
    assert!(json.contains("headerNumber"));
}

/// Helper to create a mock GraphQL response with continuity proof
fn mock_graphql_response_with_proof() -> serde_json::Value {
    serde_json::json!({
        "data": {
            "attestations": {
                "nodes": [{
                    "headerNumber": "100",
                    "root": "0x0000000000000000000000000000000000000000000000000000000000000001",
                    "digest": "0x0000000000000000000000000000000000000000000000000000000000000005",
                    "prevDigest": "0x0000000000000000000000000000000000000000000000000000000000000000",
                    "continuityProof": {
                        "blocks": [
                            {
                                "blockNumber": 100,
                                "root": "0x0000000000000000000000000000000000000000000000000000000000000001",
                                "prevDigest": "0x0000000000000000000000000000000000000000000000000000000000000000",
                                "digest": "0x0000000000000000000000000000000000000000000000000000000000000002"
                            },
                            {
                                "blockNumber": 101,
                                "root": "0x0000000000000000000000000000000000000000000000000000000000000003",
                                "prevDigest": "0x0000000000000000000000000000000000000000000000000000000000000002",
                                "digest": "0x0000000000000000000000000000000000000000000000000000000000000004"
                            }
                        ]
                    }
                }]
            }
        }
    })
}

/// Attestation entry at genesis block has an empty prevDigest and no continuity proof blocks,
/// but should still be returned successfully by get_attestation() with a valid AttestationWithProof struct.
/// This tests that edge case.
fn mock_graphl_response_from_genesis() -> serde_json::Value {
    serde_json::json!({
        "data": {
            "attestations": {
                "nodes": [{
                    "headerNumber": "0",
                    "root": "0x0000000000000000000000000000000000000000000000000000000000000000",
                    "digest": "0xdaa77426c30c02a43d9fba4e841a6556c524d47030762eb14dc4af897e605d9b",
                    "prevDigest": "",
                    "continuityProof": {
                        "blocks": []
                    }
                },
                {
                    "headerNumber": "3",
                    "root": "0x0000000000000000000000000000000000000000000000000000000000000001",
                    "digest": "0x0000000000000000000000000000000000000000000000000000000000000005",
                    "prevDigest": "0x0000000000000000000000000000000000000000000000000000000000000004",
                    "continuityProof": {
                        "blocks": [
                            {
                                "blockNumber": 1,
                                "root": "0x0000000000000000000000000000000000000000000000000000000000000001",
                                "prevDigest": "0xdaa77426c30c02a43d9fba4e841a6556c524d47030762eb14dc4af897e605d9b",
                                "digest": "0x0000000000000000000000000000000000000000000000000000000000000002"
                            },
                            {
                                "blockNumber": 2,
                                "root": "0x0000000000000000000000000000000000000000000000000000000000000003",
                                "prevDigest": "0x0000000000000000000000000000000000000000000000000000000000000002",
                                "digest": "0x0000000000000000000000000000000000000000000000000000000000000004"
                            }
                        ]
                    }
                }
                ]
            }
        }
    })
}

/// Helper to create a mock GraphQL response with empty nodes (attestation not found)
fn mock_graphql_response_empty() -> serde_json::Value {
    serde_json::json!({
        "data": {
            "attestations": {
                "nodes": []
            }
        }
    })
}

/// Helper to create a mock GraphQL response with attestation but no continuity proof
fn mock_graphql_response_no_proof() -> serde_json::Value {
    serde_json::json!({
        "data": {
            "attestations": {
                "nodes": [{
                    "headerNumber": "100",
                    "root": "0x0000000000000000000000000000000000000000000000000000000000000001",
                    "digest": "0x0000000000000000000000000000000000000000000000000000000000000005",
                    "prevDigest": "0x0000000000000000000000000000000000000000000000000000000000000000",
                    "continuityProof": null
                }]
            }
        }
    })
}

/// Helper to create a mock GraphQL error response
fn mock_graphql_error_response() -> serde_json::Value {
    serde_json::json!({
        "data": null,
        "errors": [{
            "message": "Something went wrong"
        }]
    })
}

#[tokio::test]
async fn test_get_continuity_proof_success() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_graphql_response_with_proof()))
        .mount(&mock_server)
        .await;

    let client = IndexerClient::new(mock_server.uri()).unwrap();
    let result = client.get_attestation(1, 100).await;

    assert!(result.is_ok(), "Expected Ok result, got {result:?}");
    let attestation_with_proof = result.unwrap();
    assert!(
        attestation_with_proof.is_some(),
        "Expected Some attestation"
    );

    let attestation_with_proof = attestation_with_proof.unwrap();
    assert_eq!(attestation_with_proof.block_number, 100);
    assert_eq!(attestation_with_proof.root, H256::from_low_u64_be(1));

    // Check continuity proof
    let proof = attestation_with_proof.continuity_proof.unwrap();
    // Check lower_endpoint_digest is the prev_digest of first block (0x0...0)
    assert_eq!(proof.lower_endpoint_digest, H256::zero());
    // Check we have 2 roots
    assert_eq!(proof.roots.len(), 2);
    assert_eq!(
        proof.roots[0],
        H256::from_low_u64_be(1),
        "First root should be 0x...01"
    );
    assert_eq!(
        proof.roots[1],
        H256::from_low_u64_be(3),
        "Second root should be 0x...03"
    );
}

#[tokio::test]
async fn test_get_continuity_proof_not_found() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_graphql_response_empty()))
        .mount(&mock_server)
        .await;

    let client = IndexerClient::new(mock_server.uri()).unwrap();
    let result = client.get_attestation(1, 100).await;

    assert!(result.is_ok());
    assert!(result.unwrap().is_none(), "Expected None when not found");
}

#[tokio::test]
async fn test_get_continuity_proof_at_genesis_block() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_graphl_response_from_genesis()))
        .mount(&mock_server)
        .await;

    let client = IndexerClient::new(mock_server.uri()).unwrap();
    let result = client.get_attestation(1, 2).await;

    assert!(result.is_ok());
    assert!(result.unwrap().is_some(), "Expected proof");
}

#[tokio::test]
async fn test_get_continuity_proof_attestation_without_proof() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_graphql_response_no_proof()))
        .mount(&mock_server)
        .await;

    let client = IndexerClient::new(mock_server.uri()).unwrap();
    let result = client.get_attestation(1, 100).await;

    assert!(result.is_ok());
    let attestation_with_proof = result.unwrap();
    assert!(
        attestation_with_proof.is_some(),
        "Expected Some attestation even without proof"
    );
    let attestation_with_proof = attestation_with_proof.unwrap();
    assert!(
        attestation_with_proof.continuity_proof.is_none(),
        "Expected None continuity_proof when proof is null"
    );
    assert_eq!(attestation_with_proof.block_number, 100);
}

#[tokio::test]
async fn test_get_continuity_proof_graphql_error() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(mock_graphql_error_response()))
        .mount(&mock_server)
        .await;

    let client = IndexerClient::new(mock_server.uri()).unwrap();
    let result = client.get_attestation(1, 100).await;

    assert!(result.is_err(), "Expected error on GraphQL error response");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("GraphQL errors"),
        "Error should mention GraphQL errors: {err}"
    );
}

#[tokio::test]
async fn test_get_continuity_proof_http_error() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
        .mount(&mock_server)
        .await;

    let client = IndexerClient::new(mock_server.uri()).unwrap();
    let result = client.get_attestation(1, 100).await;

    assert!(result.is_err(), "Expected error on HTTP 500");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("500"),
        "Error should mention status code: {err}"
    );
}

#[tokio::test]
async fn test_get_continuity_proof_invalid_hex() {
    let mock_server = MockServer::start().await;

    let invalid_response = serde_json::json!({
        "data": {
            "attestations": {
                "nodes": [{
                    "headerNumber": "100",
                    "root": "not-a-valid-hex",
                    "digest": "0x0000000000000000000000000000000000000000000000000000000000000001",
                    "prevDigest": null,
                    "continuityProof": {
                        "blocks": [{
                            "blockNumber": 100,
                            "root": "0x0000000000000000000000000000000000000000000000000000000000000001",
                            "prevDigest": null,
                            "digest": "0x0000000000000000000000000000000000000000000000000000000000000001"
                        }]
                    }
                }]
            }
        }
    });

    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(invalid_response))
        .mount(&mock_server)
        .await;

    let client = IndexerClient::new(mock_server.uri()).unwrap();
    let result = client.get_attestation(1, 100).await;

    assert!(result.is_err(), "Expected error on invalid hex");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("root"),
        "Error should mention root: {err}"
    );
}

#[tokio::test]
async fn test_get_continuity_proof_network_error() {
    // Use a URL that won't connect
    let client = IndexerClient::new("http://127.0.0.1:1".to_string()).unwrap();
    let result = client.get_attestation(1, 100).await;

    assert!(result.is_err(), "Expected error on network failure");
}

#[tokio::test]
async fn test_get_continuity_proof_real_response_fixture() {
    // This test uses an actual response captured from the devnet indexer
    // to ensure our deserialization matches the real schema
    let mock_server = MockServer::start().await;

    // Use actual response format from devnet (blockNumber as integer)
    let real_response = serde_json::json!({
        "data": {
            "attestations": {
                "nodes": [{
                    "headerNumber": "10034830",
                    "root": "0x77b704add5aa79dc9a617bc6fe2d61ac163b9733d9399d35420b6fb112c9030f",
                    "digest": "0xd9dad5b8da039ec98d31255a40a97f8496ba6d1690bd21e18481358c98714bfe",
                    "prevDigest": "0x963634ff1c68b602bbf38b18da3c98302f315470a7fd2bd41a34d78abb3a7b38",
                    "continuityProof": {
                        "blocks": [
                            {
                                "root": "0x77b704add5aa79dc9a617bc6fe2d61ac163b9733d9399d35420b6fb112c9030f",
                                "digest": "0xd9dad5b8da039ec98d31255a40a97f8496ba6d1690bd21e18481358c98714bfe",
                                "prevDigest": "0x963634ff1c68b602bbf38b18da3c98302f315470a7fd2bd41a34d78abb3a7b38",
                                "blockNumber": 10034821
                            },
                            {
                                "root": "0xc4d2f69b5d837c0b691b1d0adbf995f541e9de41ee02fb1c3e6b96aa91feed7e",
                                "digest": "0x50959bf79b7fa260d492d1525533673142639095316532a91f2eccfbf0819ea4",
                                "prevDigest": "0xd9dad5b8da039ec98d31255a40a97f8496ba6d1690bd21e18481358c98714bfe",
                                "blockNumber": 10034822
                            }
                        ]
                    }
                }]
            }
        }
    });

    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(real_response))
        .mount(&mock_server)
        .await;

    let client = IndexerClient::new(mock_server.uri()).unwrap();
    let result = client.get_attestation(3, 10034830).await;

    assert!(
        result.is_ok(),
        "Failed to parse real indexer response: {result:?}"
    );
    let attestation_with_proof = result.unwrap();
    assert!(
        attestation_with_proof.is_some(),
        "Expected Some attestation from real response"
    );

    let attestation_with_proof = attestation_with_proof.unwrap();
    assert_eq!(attestation_with_proof.block_number, 10034830);

    let proof = attestation_with_proof.continuity_proof.unwrap();
    assert_eq!(proof.roots.len(), 2, "Expected 2 blocks in proof");
    // Verify the lower_endpoint_digest is the prevDigest of the first block
    assert_eq!(
        format!("{:?}", proof.lower_endpoint_digest),
        "0x963634ff1c68b602bbf38b18da3c98302f315470a7fd2bd41a34d78abb3a7b38"
    );
}
