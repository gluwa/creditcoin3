//! CC3 Indexer GraphQL client for fetching attestation continuity proofs.
//!
//! This module provides a client for querying the CC3 attestations indexer
//! to pre-fetch continuity proofs when BlockAttested events are received.

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sp_core::H256;
use std::str::FromStr;
use tracing::{debug, warn};

use attestor_primitives::block::{Block, ContinuityProof};

// GraphQL query for fetching an attestation with its continuity proof.
// Note: The indexer schema uses BigFloat for numeric fields.
const ATTESTATION_QUERY: &str = r#"
query GetAttestation($chainKey: BigFloat!, $headerNumber: BigFloat!) {
    attestations(
        filter: {
            chainKey: { equalTo: $chainKey },
            headerNumber: { equalTo: $headerNumber }
        },
        first: 1
    ) {
        nodes {
            continuityProof
        }
    }
}
"#;

/// Client for querying the CC3 attestations indexer GraphQL API.
pub struct IndexerClient {
    client: Client,
    endpoint: String,
}

impl IndexerClient {
    /// Create a new indexer client with the given GraphQL endpoint.
    pub fn new(endpoint: String) -> Self {
        Self {
            client: Client::new(),
            endpoint,
        }
    }

    /// Fetch the continuity proof for an attestation by chain_key and header_number.
    ///
    /// Returns `Ok(None)` if the attestation is not yet indexed.
    /// Returns `Ok(Some(proof))` if found, or an error on network/parse failures.
    pub async fn get_continuity_proof(
        &self,
        chain_key: u64,
        header_number: u64,
    ) -> Result<Option<ContinuityProof>> {
        let query = GraphQLQuery {
            query: ATTESTATION_QUERY,
            variables: QueryVariables {
                chain_key: chain_key.to_string(),
                header_number: header_number.to_string(),
            },
        };

        let response = self
            .client
            .post(&self.endpoint)
            .json(&query)
            .send()
            .await
            .context("Failed to send GraphQL request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("GraphQL request failed with status {}: {}", status, body);
        }

        let result: GraphQLResponse = response
            .json()
            .await
            .context("Failed to parse GraphQL response")?;

        if let Some(errors) = result.errors {
            if !errors.is_empty() {
                let error_msgs: Vec<_> = errors.iter().map(|e| e.message.as_str()).collect();
                anyhow::bail!("GraphQL errors: {}", error_msgs.join(", "));
            }
        }

        let Some(data) = result.data else {
            return Ok(None);
        };

        let nodes = data.attestations.nodes;
        if nodes.is_empty() {
            debug!(
                "Attestation not found in indexer: chain_key={}, header_number={}",
                chain_key, header_number
            );
            return Ok(None);
        }

        let attestation = &nodes[0];
        let Some(ref proof_data) = attestation.continuity_proof else {
            debug!(
                "Attestation has no continuity proof: chain_key={}, header_number={}",
                chain_key, header_number
            );
            return Ok(None);
        };

        // Convert indexer blocks to attestor_primitives::block::Block
        let blocks: Result<Vec<Block>> = proof_data
            .blocks
            .iter()
            .map(|b| {
                let root = H256::from_str(&b.root).context("Invalid root hex string")?;
                let prev_digest = b
                    .prev_digest
                    .as_ref()
                    .map(|s| H256::from_str(s))
                    .transpose()
                    .context("Invalid prev_digest hex string")?
                    .unwrap_or_default();
                let digest = H256::from_str(&b.digest).context("Invalid digest hex string")?;

                Ok(Block {
                    block_number: b.block_number,
                    root,
                    prev_digest,
                    digest,
                })
            })
            .collect();

        let blocks = blocks?;

        if blocks.is_empty() {
            warn!(
                "Continuity proof has no blocks: chain_key={}, header_number={}",
                chain_key, header_number
            );
            return Ok(None);
        }

        // Build ContinuityProof from blocks
        // The lower_endpoint_digest is the prev_digest of the first block
        let lower_endpoint_digest = blocks[0].prev_digest;
        let roots: Vec<H256> = blocks.iter().map(|b| b.root).collect();

        Ok(Some(ContinuityProof {
            lower_endpoint_digest,
            roots,
        }))
    }
}

// GraphQL request/response types specific to this query.
// These are not shared with usc_audit_automation because that module
// queries different fields and has different response shapes.

#[derive(Serialize)]
struct GraphQLQuery<'a> {
    query: &'a str,
    variables: QueryVariables,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct QueryVariables {
    chain_key: String,
    header_number: String,
}

#[derive(Deserialize)]
struct GraphQLResponse {
    data: Option<ResponseData>,
    errors: Option<Vec<GraphQLError>>,
}

#[derive(Deserialize)]
struct GraphQLError {
    message: String,
}

#[derive(Deserialize)]
struct ResponseData {
    attestations: AttestationsConnection,
}

#[derive(Deserialize)]
struct AttestationsConnection {
    nodes: Vec<AttestationNode>,
}

/// Attestation node from GraphQL response.
/// Only includes the continuityProof field we need for this use case.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttestationNode {
    continuity_proof: Option<ContinuityProofData>,
}

/// The continuityProof JSON blob from the indexer.
/// Contains an array of blocks that form the continuity proof.
#[derive(Deserialize)]
struct ContinuityProofData {
    blocks: Vec<ContinuityBlockData>,
}

/// Individual block within a continuity proof.
/// Field names match the CC3 indexer schema (camelCase).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContinuityBlockData {
    block_number: u64,
    root: String,
    prev_digest: Option<String>,
    digest: String,
}

#[cfg(test)]
mod tests {
    use super::*;
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
            .respond_with(
                ResponseTemplate::new(200).set_body_json(mock_graphql_response_with_proof()),
            )
            .mount(&mock_server)
            .await;

        let client = IndexerClient::new(mock_server.uri());
        let result = client.get_continuity_proof(1, 100).await;

        assert!(result.is_ok(), "Expected Ok result, got {result:?}");
        let proof = result.unwrap();
        assert!(proof.is_some(), "Expected Some proof");

        let proof = proof.unwrap();
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

        let client = IndexerClient::new(mock_server.uri());
        let result = client.get_continuity_proof(1, 100).await;

        assert!(result.is_ok());
        assert!(result.unwrap().is_none(), "Expected None when not found");
    }

    #[tokio::test]
    async fn test_get_continuity_proof_attestation_without_proof() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(mock_graphql_response_no_proof()),
            )
            .mount(&mock_server)
            .await;

        let client = IndexerClient::new(mock_server.uri());
        let result = client.get_continuity_proof(1, 100).await;

        assert!(result.is_ok());
        assert!(
            result.unwrap().is_none(),
            "Expected None when proof is null"
        );
    }

    #[tokio::test]
    async fn test_get_continuity_proof_graphql_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_graphql_error_response()))
            .mount(&mock_server)
            .await;

        let client = IndexerClient::new(mock_server.uri());
        let result = client.get_continuity_proof(1, 100).await;

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

        let client = IndexerClient::new(mock_server.uri());
        let result = client.get_continuity_proof(1, 100).await;

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
                        "continuityProof": {
                            "blocks": [{
                                "blockNumber": 100,
                                "root": "not-a-valid-hex",
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

        let client = IndexerClient::new(mock_server.uri());
        let result = client.get_continuity_proof(1, 100).await;

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
        let client = IndexerClient::new("http://127.0.0.1:1".to_string());
        let result = client.get_continuity_proof(1, 100).await;

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

        let client = IndexerClient::new(mock_server.uri());
        let result = client.get_continuity_proof(3, 10034830).await;

        assert!(
            result.is_ok(),
            "Failed to parse real indexer response: {result:?}"
        );
        let proof = result.unwrap();
        assert!(proof.is_some(), "Expected Some proof from real response");

        let proof = proof.unwrap();
        assert_eq!(proof.roots.len(), 2, "Expected 2 blocks in proof");
        // Verify the lower_endpoint_digest is the prevDigest of the first block
        assert_eq!(
            format!("{:?}", proof.lower_endpoint_digest),
            "0x963634ff1c68b602bbf38b18da3c98302f315470a7fd2bd41a34d78abb3a7b38"
        );
    }
}
