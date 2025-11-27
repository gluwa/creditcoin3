use anyhow::Result;
use attestor_primitives::Query;
use continuity::{builder::ContinuityBuilder, config::ContinuityConfig};
use continuity_mocks::make_mock_providers;

#[tokio::test]
async fn builder_builds_trimmed_continuity_chain_for_single_query() -> Result<()> {
    let chain_key = 2;
    let config = ContinuityConfig {
        chain_key,
        cc3_rpc_url: "http://localhost:1234".to_string(),
        cc3_key: "//Alice".to_string(),
        eth_rpc_url: "http://localhost:5678".to_string(),
    };

    let (cc, eth) = make_mock_providers(chain_key);
    let builder = ContinuityBuilder::new_with_providers(config, cc, eth);

    let query_height = 10;
    let query = Query {
        chain_id: chain_key,
        height: query_height,
        layout_segments: vec![],
    };
    let proof = builder.build_for_single_query(&query).await?;

    // Expect chain starts at queryHeight - 1 (9) and ends at next attestation (15)
    let first = proof.blocks.first().expect("non-empty continuity chain");
    let last = proof.blocks.last().expect("non-empty continuity chain");

    assert_eq!(
        first.block_number,
        query_height - 1,
        "continuity chain must start at queryHeight-1"
    );
    assert_eq!(
        last.block_number, 15,
        "continuity chain must end at next attestation height"
    );
    assert!(
        proof.blocks.len() <= ((15 - (query_height - 1) + 1) as usize),
        "chain length within expected bounds"
    );

    Ok(())
}
