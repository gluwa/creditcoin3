use anyhow::Result;
use continuity::{
    builder::ContinuityBuilder, config::ContinuityConfig, mocks::make_mock_providers,
};

#[tokio::test]
async fn builder_builds_trimmed_continuity_chain_for_single_query() -> Result<()> {
    let chain_key = 2;
    let config = ContinuityConfig::builder()
        .cc3_rpc_url("http://localhost:1234")
        .eth_rpc_url("http://localhost:5678")
        .chain_key(chain_key)
        .attestation_interval(10)
        .checkpoint_interval(10)
        .build();

    let (cc, eth) = make_mock_providers(chain_key);
    let builder = ContinuityBuilder::new_with_providers(config, cc, eth);

    let query_height = 15; // Between attestations at 10 and 20
    let (lower_attestation, upper_attestation, _) =
        builder.get_endpoints(&[query_height], None).await?;
    let proof = builder
        .build_for_single_query(query_height, lower_attestation, upper_attestation)
        .await?;

    // Expect chain starts at queryHeight (15) and ends at next attestation (20)
    let first = proof.blocks.first().expect("non-empty continuity chain");
    let last = proof.blocks.last().expect("non-empty continuity chain");

    assert_eq!(
        first.block_number, query_height,
        "continuity chain must start at queryHeight (query at index 0)"
    );
    assert_eq!(
        last.block_number, 20,
        "continuity chain must end at next attestation height"
    );
    assert!(
        proof.blocks.len() <= ((20 - query_height + 1) as usize),
        "chain length within expected bounds"
    );

    Ok(())
}
