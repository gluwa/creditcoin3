//! GraphQL queries for fetching attestation metadata from the indexer.

/// GraphQL query for fetching an attestation with its continuity proof by header_number.
/// Note: The indexer schema uses BigFloat for numeric fields.
pub const ATTESTATION_BY_HEADER_QUERY: &str = r#"
query GetAttestation($chainKey: BigFloat!, $headerNumber: BigFloat!) {
    attestations(
        filter: {
            chainKey: { equalTo: $chainKey },
            headerNumber: { equalTo: $headerNumber }
        },
        first: 1
    ) {
        nodes {
            root
            digest
            prevDigest
            continuityProof
        }
    }
}
"#;

/// Query to find attestations at or before a block number (returns full attestation data)
pub const ATTESTATION_BEFORE_OR_AT_QUERY: &str = r#"
query GetAttestationBeforeOrAt($chainKey: BigFloat!, $headerNumber: BigFloat!) {
    attestations(
        filter: {
            chainKey: { equalTo: $chainKey },
            headerNumber: { lessThanOrEqualTo: $headerNumber }
        },
        first: 1
        orderBy: HEADER_NUMBER_DESC
    ) {
        nodes {
            headerNumber
            root
            digest
            prevDigest
            continuityProof
        }
    }
}
"#;

/// Query to find attestations after a block number (returns full attestation data)
pub const ATTESTATION_AFTER_QUERY: &str = r#"
query GetAttestationAfter($chainKey: BigFloat!, $headerNumber: BigFloat!) {
    attestations(
        filter: {
            chainKey: { equalTo: $chainKey },
            headerNumber: { greaterThan: $headerNumber }
        },
        first: 1
        orderBy: HEADER_NUMBER_ASC
    ) {
        nodes {
            headerNumber
            root
            digest
            prevDigest
            continuityProof
        }
    }
}
"#;

/// Query to get the last (most recent) attestation (returns full attestation data)
pub const LAST_ATTESTATION_QUERY: &str = r#"
query GetLastAttestation($chainKey: BigFloat!) {
    attestations(
        filter: {
            chainKey: { equalTo: $chainKey }
        },
        first: 1
        orderBy: HEADER_NUMBER_DESC
    ) {
        nodes {
            headerNumber
            root
            digest
            prevDigest
            continuityProof
        }
    }
}
"#;

/// Query to get all attestations in a range (for checkpoint-spanning proofs)
pub const ATTESTATIONS_IN_RANGE_QUERY: &str = r#"
query GetAttestationsInRange($chainKey: BigFloat!, $minBlock: BigFloat!, $maxBlock: BigFloat!) {
    attestations(
        filter: {
            chainKey: { equalTo: $chainKey },
            headerNumber: { greaterThanOrEqualTo: $minBlock, lessThanOrEqualTo: $maxBlock }
        },
        orderBy: HEADER_NUMBER_ASC
    ) {
        nodes {
            headerNumber
            root
            digest
            prevDigest
            continuityProof
        }
    }
}
"#;

/// Query to get all checkpoints for a chain, sorted by block number descending (newest first)
pub const CHECKPOINTS_QUERY: &str = r#"
query GetCheckpoints($chainKey: BigFloat!) {
    checkpoints(
        filter: {
            chainKey: { equalTo: $chainKey }
        },
        orderBy: BLOCK_NUMBER_DESC
    ) {
        nodes {
            blockNumber
            digest
        }
    }
}
"#;

/// Query to get a specific checkpoint by block number
pub const CHECKPOINT_BY_BLOCK_QUERY: &str = r#"
query GetCheckpointByBlock($chainKey: BigFloat!, $blockNumber: BigFloat!) {
    checkpoints(
        filter: {
            chainKey: { equalTo: $chainKey },
            blockNumber: { equalTo: $blockNumber }
        },
        first: 1
    ) {
        nodes {
            blockNumber
            digest
        }
    }
}
"#;

/// Query to get the last (most recent) checkpoint for a chain
pub const LAST_CHECKPOINT_QUERY: &str = r#"
query GetLastCheckpoint($chainKey: BigFloat!) {
    checkpoints(
        filter: {
            chainKey: { equalTo: $chainKey }
        },
        first: 1
        orderBy: BLOCK_NUMBER_DESC
    ) {
        nodes {
            blockNumber
            digest
        }
    }
}
"#;

/// Query to get checkpoints in a range around a query height
/// Fetches checkpoints before and after the query to find boundaries
///
/// Note: The `checkpointsBefore` filter uses `lessThanOrEqualTo: $queryHeight` to get checkpoints
/// at or before the query. The `checkpointsAfter` filter uses `greaterThan: $queryHeight` to get
/// checkpoints strictly after the query. Both are bounded by the range ($minBlock to $maxBlock).
pub const CHECKPOINTS_IN_RANGE_QUERY: &str = r#"
query GetCheckpointsInRange($chainKey: BigFloat!, $minBlock: BigFloat!, $maxBlock: BigFloat!, $queryHeight: BigFloat!) {
    checkpointsBefore: checkpoints(
        filter: {
            chainKey: { equalTo: $chainKey },
            blockNumber: { greaterThanOrEqualTo: $minBlock, lessThanOrEqualTo: $queryHeight }
        },
        first: 10
        orderBy: BLOCK_NUMBER_DESC
    ) {
        nodes {
            blockNumber
            digest
        }
    }
    checkpointsAfter: checkpoints(
        filter: {
            chainKey: { equalTo: $chainKey },
            blockNumber: { greaterThan: $queryHeight, lessThanOrEqualTo: $maxBlock }
        },
        first: 10
        orderBy: BLOCK_NUMBER_ASC
    ) {
        nodes {
            blockNumber
            digest
        }
    }
}
"#;
