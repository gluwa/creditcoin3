//! GraphQL client for querying the CC3 attestations indexer

use anyhow::Result;
use reqwest::Client;
use tracing::{debug, info};

use crate::error::IndexerError;
use crate::queries::*;
use crate::types::{AttestationWithProof, *};
use crate::utils::{parse_attestation_node, parse_attestation_node_full, parse_checkpoint_node};

/// Client for querying the CC3 attestations indexer GraphQL API.
pub struct IndexerClient {
    client: Client,
    endpoint: String,
}

impl IndexerClient {
    /// Create a new indexer client with the given GraphQL endpoint.
    pub fn new(endpoint: String) -> Result<Self, IndexerError> {
        if endpoint.is_empty() {
            return Err(IndexerError::InvalidEndpoint(
                "endpoint cannot be empty".to_string(),
            ));
        }

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| IndexerError::ClientBuild(e.to_string()))?;

        info!("Initialized indexer client with endpoint: {}", endpoint);

        Ok(Self { client, endpoint })
    }

    /// Helper to execute GraphQL query and handle common response parsing
    async fn execute_graphql_query<TQuery, TResponse>(
        &self,
        query: GraphQLQueryWrapper<TQuery>,
    ) -> Result<GraphQLResponseWrapper<TResponse>, IndexerError>
    where
        TQuery: serde::Serialize,
        TResponse: serde::de::DeserializeOwned,
    {
        let response = self.client.post(&self.endpoint).json(&query).send().await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_else(|_| String::new());
            return Err(IndexerError::GraphQLRequestFailed { status, body });
        }

        let result: GraphQLResponseWrapper<TResponse> = response.json().await?;

        if let Some(ref errors) = result.errors {
            if !errors.is_empty() {
                let error_msgs: Vec<_> = errors.iter().map(|e| e.message.as_str()).collect();
                return Err(IndexerError::GraphQLErrors(error_msgs.join(", ")));
            }
        }

        Ok(result)
    }

    /// Fetch attestation with both metadata and proof in a single GraphQL query.
    ///
    /// Returns `Ok(None)` if the attestation is not yet indexed.
    /// Returns `Ok(Some(proof))` if found, or an error on network/parse failures.
    pub async fn get_attestation(
        &self,
        chain_key: u64,
        header_number: u64,
    ) -> Result<Option<AttestationWithProof>, IndexerError> {
        self.get_attestation_with_query(ATTESTATION_BY_HEADER_QUERY, chain_key, Some(header_number))
            .await
    }

    /// Fetch the attestation with proof for a specific attestation block number.
    /// Alias for `get_attestation` for compatibility.
    pub async fn get_continuity_blocks(
        &self,
        chain_key: u64,
        attestation_header_number: u64,
    ) -> Result<Option<AttestationWithProof>, IndexerError> {
        self.get_attestation(chain_key, attestation_header_number)
            .await
    }

    /// Find the attestation at or before the given block number.
    pub async fn find_attestation_before_or_at(
        &self,
        chain_key: u64,
        block_number: u64,
    ) -> Result<Option<AttestationWithProof>, IndexerError> {
        self.get_attestation_with_query(
            ATTESTATION_BEFORE_OR_AT_QUERY,
            chain_key,
            Some(block_number),
        )
        .await
    }

    /// Find the attestation after the given block number.
    pub async fn find_attestation_after(
        &self,
        chain_key: u64,
        block_number: u64,
    ) -> Result<Option<AttestationWithProof>, IndexerError> {
        // Don't pass header_number to parse_attestation_node - we want to use the actual
        // headerNumber from the GraphQL response (which should be > block_number)
        let variables = serde_json::json!({
            "chainKey": chain_key.to_string(),
            "headerNumber": block_number.to_string()
        });

        let graphql_query = serde_json::json!({
            "query": ATTESTATION_AFTER_QUERY,
            "variables": variables
        });

        let response = self
            .client
            .post(&self.endpoint)
            .json(&graphql_query)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_else(|_| String::new());
            return Err(IndexerError::GraphQLRequestFailed { status, body });
        }

        let result: GraphQLResponse = response.json().await?;

        if let Some(errors) = result.errors {
            if !errors.is_empty() {
                let error_msgs: Vec<_> = errors.iter().map(|e| e.message.as_str()).collect();
                return Err(IndexerError::GraphQLErrors(error_msgs.join(", ")));
            }
        }

        let Some(data) = result.data else {
            return Ok(None);
        };

        let nodes = data.attestations.nodes;
        if nodes.is_empty() {
            return Ok(None);
        }

        let attestation = &nodes[0];
        // Pass None for header_number so parse_attestation_node uses the value from the node
        Ok(Some(parse_attestation_node(attestation, None)?))
    }

    /// Get the last (most recent) attestation for a chain.
    pub async fn get_last_attestation(
        &self,
        chain_key: u64,
    ) -> Result<Option<AttestationWithProof>, IndexerError> {
        self.get_attestation_with_query(LAST_ATTESTATION_QUERY, chain_key, None)
            .await
    }

    /// Internal helper to query attestation with a specific GraphQL query.
    async fn get_attestation_with_query(
        &self,
        query: &'static str,
        chain_key: u64,
        header_number: Option<u64>,
    ) -> Result<Option<AttestationWithProof>, IndexerError> {
        let variables = if let Some(header_number) = header_number {
            serde_json::json!({
                "chainKey": chain_key.to_string(),
                "headerNumber": header_number.to_string()
            })
        } else {
            serde_json::json!({
                "chainKey": chain_key.to_string()
            })
        };

        let graphql_query = serde_json::json!({
            "query": query,
            "variables": variables
        });

        let response = self
            .client
            .post(&self.endpoint)
            .json(&graphql_query)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_else(|_| String::new());
            return Err(IndexerError::GraphQLRequestFailed { status, body });
        }

        let result: GraphQLResponse = response.json().await?;

        if let Some(errors) = result.errors {
            if !errors.is_empty() {
                let error_msgs: Vec<_> = errors.iter().map(|e| e.message.as_str()).collect();
                return Err(IndexerError::GraphQLErrors(error_msgs.join(", ")));
            }
        }

        let Some(data) = result.data else {
            return Ok(None);
        };

        let nodes = data.attestations.nodes;
        if nodes.is_empty() {
            return Ok(None);
        }

        let attestation = &nodes[0];
        Ok(Some(parse_attestation_node(attestation, header_number)?))
    }

    /// Fetch all attestations in a block range (optimized batch query for checkpoint-spanning proofs).
    /// Returns attestations with their continuity proofs, ordered by header_number ASC.
    pub async fn get_attestations_in_range(
        &self,
        chain_key: u64,
        min_block: u64,
        max_block: u64,
    ) -> Result<Vec<AttestationWithProof>, IndexerError> {
        let query = GraphQLQueryWrapper {
            query: ATTESTATIONS_IN_RANGE_QUERY,
            variables: RangeQueryVariables {
                chain_key: chain_key.to_string(),
                min_block: min_block.to_string(),
                max_block: max_block.to_string(),
            },
        };

        let response = self.client.post(&self.endpoint).json(&query).send().await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_else(|_| String::new());
            return Err(IndexerError::GraphQLRequestFailed { status, body });
        }

        let range_result: GraphQLResponseWrapper<AttestationsRangeResponseData> =
            response.json().await?;

        if let Some(errors) = range_result.errors {
            if !errors.is_empty() {
                let error_msgs: Vec<_> = errors.iter().map(|e| e.message.as_str()).collect();
                return Err(IndexerError::GraphQLErrors(error_msgs.join(", ")));
            }
        }

        let Some(data) = range_result.data else {
            return Ok(Vec::new());
        };

        let mut attestations = Vec::new();

        for node in data.attestations.nodes {
            attestations.push(parse_attestation_node_full(&node)?);
        }

        debug!(
            fetched_attestations = attestations.len(),
            "Batch fetched attestations in range"
        );

        Ok(attestations)
    }

    /// Fetch all checkpoints for a chain from the indexer.
    ///
    /// Returns checkpoints sorted by block number descending (newest first).
    pub async fn get_checkpoints_for_chain(
        &self,
        chain_key: u64,
    ) -> Result<Vec<attestor_primitives::AttestationCheckpoint>, IndexerError> {
        let result = self
            .execute_graphql_query::<CheckpointQueryVariables, CheckpointResponseData>(
                GraphQLQueryWrapper {
                    query: CHECKPOINTS_QUERY,
                    variables: CheckpointQueryVariables {
                        chain_key: chain_key.to_string(),
                    },
                },
            )
            .await?;

        let Some(data) = result.data else {
            return Ok(Vec::new());
        };

        let mut checkpoints = Vec::new();
        for node in data.checkpoints.nodes {
            checkpoints.push(parse_checkpoint_node(&node)?);
        }

        // Checkpoints are already sorted DESC by the query, but ensure it's correct
        checkpoints.sort_by_key(|c| std::cmp::Reverse(c.block_number));

        info!(
            chain_key,
            checkpoint_count = checkpoints.len(),
            first_checkpoint = checkpoints.first().map(|c| c.block_number),
            last_checkpoint = checkpoints.last().map(|c| c.block_number),
            "Fetched and sorted checkpoints from indexer"
        );

        Ok(checkpoints)
    }

    /// Get a specific checkpoint by block number.
    pub async fn get_checkpoint_by_height(
        &self,
        chain_key: u64,
        block_number: u64,
    ) -> Result<Option<attestor_primitives::AttestationCheckpoint>, IndexerError> {
        let result = self
            .execute_graphql_query::<CheckpointByBlockVariables, CheckpointResponseData>(
                GraphQLQueryWrapper {
                    query: CHECKPOINT_BY_BLOCK_QUERY,
                    variables: CheckpointByBlockVariables {
                        chain_key: chain_key.to_string(),
                        block_number: block_number.to_string(),
                    },
                },
            )
            .await?;

        let Some(data) = result.data else {
            return Ok(None);
        };

        let Some(node) = data.checkpoints.nodes.first() else {
            return Ok(None);
        };

        Ok(Some(parse_checkpoint_node(node)?))
    }

    /// Get checkpoints around a query height (before and after).
    /// This is more efficient than fetching all checkpoints when we only need a few around the query.
    pub async fn get_checkpoints_around_height(
        &self,
        chain_key: u64,
        query_height: u64,
        max_range: u64,
    ) -> Result<Vec<attestor_primitives::AttestationCheckpoint>, IndexerError> {
        // Calculate range: fetch checkpoints from (query_height - max_range) to (query_height + max_range)
        let min_block = query_height.saturating_sub(max_range);
        let max_block = query_height.saturating_add(max_range);

        let result = self
            .execute_graphql_query::<RangeQueryVariables, CheckpointsInRangeData>(
                GraphQLQueryWrapper {
                    query: crate::queries::CHECKPOINTS_IN_RANGE_QUERY,
                    variables: RangeQueryVariables {
                        chain_key: chain_key.to_string(),
                        min_block: min_block.to_string(),
                        max_block: max_block.to_string(),
                    },
                },
            )
            .await?;

        let Some(data) = result.data else {
            return Ok(Vec::new());
        };

        let mut checkpoints = Vec::new();

        // Process checkpoints before (already sorted DESC)
        for node in data.checkpoints_before.nodes {
            checkpoints.push(parse_checkpoint_node(&node)?);
        }

        // Process checkpoints after (already sorted ASC)
        for node in data.checkpoints_after.nodes {
            checkpoints.push(parse_checkpoint_node(&node)?);
        }

        // Sort by block number descending (newest first)
        checkpoints.sort_by_key(|c| std::cmp::Reverse(c.block_number));

        debug!(
            chain_key,
            query_height,
            checkpoint_count = checkpoints.len(),
            "Fetched checkpoints around query height"
        );

        Ok(checkpoints)
    }

    /// Get the last (most recent) checkpoint for a chain.
    pub async fn get_last_checkpoint(
        &self,
        chain_key: u64,
    ) -> Result<Option<attestor_primitives::AttestationCheckpoint>, IndexerError> {
        let result = self
            .execute_graphql_query::<CheckpointQueryVariables, CheckpointResponseData>(
                GraphQLQueryWrapper {
                    query: LAST_CHECKPOINT_QUERY,
                    variables: CheckpointQueryVariables {
                        chain_key: chain_key.to_string(),
                    },
                },
            )
            .await?;

        let Some(data) = result.data else {
            return Ok(None);
        };

        let Some(node) = data.checkpoints.nodes.first() else {
            return Ok(None);
        };

        Ok(Some(parse_checkpoint_node(node)?))
    }
}
