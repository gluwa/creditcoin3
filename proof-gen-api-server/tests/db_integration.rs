use anyhow::Result;
use attestor_primitives::AttestationCheckpoint;
use bigdecimal::BigDecimal;
use diesel_async::{AsyncConnection, AsyncPgConnection};
use sp_core::H256;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

use proof_gen_api_server::db::{continuity_blocks, models::*};

/// Helper: set up test database with migrations and return connection
async fn setup_test_db() -> Result<(testcontainers::ContainerAsync<Postgres>, AsyncPgConnection)> {
    let container = Postgres::default().start().await?;
    let port = container.get_host_port_ipv4(5432).await?;
    // secretlint-disable
    let db_url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    // secretlint-enable

    run_migrations(&db_url).await?;
    let conn = AsyncPgConnection::establish(&db_url).await?;

    Ok((container, conn))
}

/// Helper: run diesel migrations
async fn run_migrations(db_url: &str) -> Result<()> {
    use diesel_async::async_connection_wrapper::AsyncConnectionWrapper;
    use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

    const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./migrations");

    let db_url = db_url.to_string();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let mut wrapper =
            <AsyncConnectionWrapper<AsyncPgConnection> as diesel::Connection>::establish(&db_url)
                .map_err(|e| anyhow::anyhow!("Failed to establish connection: {e}"))?;
        wrapper
            .run_pending_migrations(MIGRATIONS)
            .map_err(|e| anyhow::anyhow!("Failed to run migrations: {e}"))?;
        Ok(())
    })
    .await??;

    Ok(())
}

#[cfg_attr(not(feature = "integration-tests"), ignore)]
#[tokio::test]
async fn test_attestation_storage_and_retrieval() -> Result<()> {
    let (_container, mut conn) = setup_test_db().await?;

    // Create test attestations using ContinuityBlockItem
    let attestation1 = ContinuityBlockItem::from_attestation(
        1,
        100,
        "0x1234567890abcdef111111111111111111111111111111111111111111111111".to_string(),
    );

    let attestation2 = ContinuityBlockItem::from_attestation(
        1,
        200,
        "0x2234567890abcdef222222222222222222222222222222222222222222222222".to_string(),
    );

    // Insert attestations
    continuity_blocks::insert_attestation(&mut conn, &attestation1).await?;
    continuity_blocks::insert_attestation(&mut conn, &attestation2).await?;

    // Test get_highest_attestation_at_or_before
    let result = continuity_blocks::get_highest_attestation_at_or_before(&mut conn, 1, 150).await?;
    assert!(result.is_some());
    let stored = result.unwrap();
    assert_eq!(stored.chain_key, attestation1.chain_key);
    assert_eq!(stored.header_number, attestation1.header_number);
    assert_eq!(stored.digest, attestation1.digest);

    // Test get_lowest_attestation_at_or_after
    let result = continuity_blocks::get_lowest_attestation_at_or_after(&mut conn, 1, 150).await?;
    assert!(result.is_some());
    let stored = result.unwrap();
    assert_eq!(stored.chain_key, attestation2.chain_key);
    assert_eq!(stored.header_number, attestation2.header_number);
    assert_eq!(stored.digest, attestation2.digest);

    // Test no result cases
    let result = continuity_blocks::get_highest_attestation_at_or_before(&mut conn, 1, 50).await?;
    assert!(result.is_none());

    let result = continuity_blocks::get_lowest_attestation_at_or_after(&mut conn, 1, 250).await?;
    assert!(result.is_none());

    Ok(())
}

#[cfg_attr(not(feature = "integration-tests"), ignore)]
#[tokio::test]
async fn test_checkpoint_storage_and_retrieval() -> Result<()> {
    let (_container, mut conn) = setup_test_db().await?;

    // Create test checkpoints
    let checkpoint1 = AttestationCheckpoint {
        block_number: 1000,
        digest: H256::from_low_u64_be(1000),
    };

    let checkpoint2 = AttestationCheckpoint {
        block_number: 2000,
        digest: H256::from_low_u64_be(2000),
    };

    // Insert checkpoints
    let checkpoint_item1 = ContinuityBlockItem::from_checkpoint(1, &checkpoint1);
    let checkpoint_item2 = ContinuityBlockItem::from_checkpoint(1, &checkpoint2);
    continuity_blocks::insert_checkpoint(&mut conn, &checkpoint_item1).await?;
    continuity_blocks::insert_checkpoint(&mut conn, &checkpoint_item2).await?;

    // Test get_highest_checkpoint_at_or_before
    let result = continuity_blocks::get_highest_checkpoint_at_or_before(&mut conn, 1, 1500).await?;
    assert!(result.is_some());
    let stored = result.unwrap();
    assert_eq!(stored.chain_key, 1);
    assert_eq!(stored.header_number.to_string(), "1000");

    // Test get_lowest_checkpoint_at_or_after
    let result = continuity_blocks::get_lowest_checkpoint_at_or_after(&mut conn, 1, 1500).await?;
    assert!(result.is_some());
    let stored = result.unwrap();
    assert_eq!(stored.chain_key, 1);
    assert_eq!(stored.header_number.to_string(), "2000");

    // Test no result cases
    let result = continuity_blocks::get_highest_checkpoint_at_or_before(&mut conn, 1, 500).await?;
    assert!(result.is_none());

    let result = continuity_blocks::get_lowest_checkpoint_at_or_after(&mut conn, 1, 2500).await?;
    assert!(result.is_none());

    Ok(())
}

#[cfg_attr(not(feature = "integration-tests"), ignore)]
#[tokio::test]
async fn test_duplicate_attestation_same_location_handling() -> Result<()> {
    let (_container, mut conn) = setup_test_db().await?;

    let attestation1 = ContinuityBlockItem::from_attestation(
        1,
        100,
        "0x1234567890abcdef111111111111111111111111111111111111111111111111".to_string(),
    );

    // Insert the first attestation
    continuity_blocks::insert_attestation(&mut conn, &attestation1).await?;

    // Attempt to insert another attestation at same (chain_key, header_number) with different digest
    // This tests the partial unique index with filter_target - should be silently ignored via do_nothing()
    let attestation2 = ContinuityBlockItem::from_attestation(
        1,
        100,
        "0xabcdef1234567890222222222222222222222222222222222222222222222222".to_string(),
    );
    continuity_blocks::insert_attestation(&mut conn, &attestation2).await?;

    // Verify only the first record exists with original values (conflict was ignored)
    let result = continuity_blocks::get_highest_attestation_at_or_before(&mut conn, 1, 100).await?;
    assert!(result.is_some());
    let stored = result.unwrap();
    assert_eq!(stored.chain_key, attestation1.chain_key);
    assert_eq!(stored.header_number, attestation1.header_number);
    assert_eq!(stored.digest, attestation1.digest);
    assert_ne!(
        stored.digest, attestation2.digest,
        "Second attestation should have been ignored"
    );

    Ok(())
}

#[cfg_attr(not(feature = "integration-tests"), ignore)]
#[tokio::test]
async fn test_duplicate_checkpoint_same_location_handling() -> Result<()> {
    let (_container, mut conn) = setup_test_db().await?;

    let checkpoint1 = AttestationCheckpoint {
        block_number: 1000,
        digest: H256::from_low_u64_be(12345),
    };

    // Insert the first checkpoint
    let checkpoint_item1 = ContinuityBlockItem::from_checkpoint(1, &checkpoint1);
    continuity_blocks::insert_checkpoint(&mut conn, &checkpoint_item1).await?;

    // Attempt to insert another checkpoint at same (chain_key, header_number) with different digest
    // This tests the partial unique index with filter_target - should be silently ignored via do_nothing()
    let checkpoint2 = AttestationCheckpoint {
        block_number: 1000,
        digest: H256::from_low_u64_be(99999),
    };
    let checkpoint_item2 = ContinuityBlockItem::from_checkpoint(1, &checkpoint2);
    continuity_blocks::insert_checkpoint(&mut conn, &checkpoint_item2).await?;

    // Verify only the first record exists with original values (conflict was ignored)
    let result = continuity_blocks::get_highest_checkpoint_at_or_before(&mut conn, 1, 1000).await?;
    assert!(result.is_some());
    let stored = result.unwrap();
    assert_eq!(stored.chain_key, 1);
    assert_eq!(
        stored.header_number,
        BigDecimal::from(checkpoint1.block_number)
    );
    assert_eq!(stored.digest, format!("0x{:x}", checkpoint1.digest));
    assert_ne!(
        stored.digest,
        format!("0x{:x}", checkpoint2.digest),
        "Second checkpoint should have been ignored"
    );

    Ok(())
}

#[cfg_attr(not(feature = "integration-tests"), ignore)]
#[tokio::test]
async fn test_attestation_boundary_exact_match() -> Result<()> {
    let (_container, mut conn) = setup_test_db().await?;

    // Create test attestations
    let attestation1 = ContinuityBlockItem::from_attestation(
        1,
        100,
        "0x1234567890abcdef111111111111111111111111111111111111111111111111".to_string(),
    );

    let attestation2 = ContinuityBlockItem::from_attestation(
        1,
        200,
        "0x2234567890abcdef222222222222222222222222222222222222222222222222".to_string(),
    );

    let attestation3 = ContinuityBlockItem::from_attestation(
        1,
        300,
        "0x3234567890abcdef333333333333333333333333333333333333333333333333".to_string(),
    );

    continuity_blocks::insert_attestation(&mut conn, &attestation1).await?;
    continuity_blocks::insert_attestation(&mut conn, &attestation2).await?;
    continuity_blocks::insert_attestation(&mut conn, &attestation3).await?;

    // Test get_highest_attestation_at_or_before with exact match
    // Querying at 200 should return attestation2 (inclusive)
    let result = continuity_blocks::get_highest_attestation_at_or_before(&mut conn, 1, 200).await?;
    assert!(result.is_some());
    let stored = result.unwrap();
    assert_eq!(stored.header_number, BigDecimal::from(200));
    assert_eq!(stored.digest, attestation2.digest);

    // Test get_lowest_attestation_at_or_after with exact match
    // Querying at 200 should return attestation2 (inclusive)
    let result = continuity_blocks::get_lowest_attestation_at_or_after(&mut conn, 1, 200).await?;
    assert!(result.is_some());
    let stored = result.unwrap();
    assert_eq!(stored.header_number, BigDecimal::from(200));
    assert_eq!(stored.digest, attestation2.digest);

    // Test boundary cases: at_or_before at exactly the lowest value
    let result = continuity_blocks::get_highest_attestation_at_or_before(&mut conn, 1, 100).await?;
    assert!(result.is_some());
    let stored = result.unwrap();
    assert_eq!(stored.header_number, BigDecimal::from(100));
    assert_eq!(stored.digest, attestation1.digest);

    // Test boundary cases: at_or_after at exactly the highest value
    let result = continuity_blocks::get_lowest_attestation_at_or_after(&mut conn, 1, 300).await?;
    assert!(result.is_some());
    let stored = result.unwrap();
    assert_eq!(stored.header_number, BigDecimal::from(300));
    assert_eq!(stored.digest, attestation3.digest);

    Ok(())
}

#[cfg_attr(not(feature = "integration-tests"), ignore)]
#[tokio::test]
async fn test_checkpoint_boundary_exact_match() -> Result<()> {
    let (_container, mut conn) = setup_test_db().await?;

    // Create test checkpoints
    let checkpoint1 = AttestationCheckpoint {
        block_number: 1000,
        digest: H256::from_low_u64_be(1000),
    };

    let checkpoint2 = AttestationCheckpoint {
        block_number: 2000,
        digest: H256::from_low_u64_be(2000),
    };

    let checkpoint3 = AttestationCheckpoint {
        block_number: 3000,
        digest: H256::from_low_u64_be(3000),
    };

    let checkpoint_item1 = ContinuityBlockItem::from_checkpoint(1, &checkpoint1);
    let checkpoint_item2 = ContinuityBlockItem::from_checkpoint(1, &checkpoint2);
    let checkpoint_item3 = ContinuityBlockItem::from_checkpoint(1, &checkpoint3);
    continuity_blocks::insert_checkpoint(&mut conn, &checkpoint_item1).await?;
    continuity_blocks::insert_checkpoint(&mut conn, &checkpoint_item2).await?;
    continuity_blocks::insert_checkpoint(&mut conn, &checkpoint_item3).await?;

    // Test get_highest_checkpoint_at_or_before with exact match
    // Querying at 2000 should return checkpoint2 (inclusive)
    let result = continuity_blocks::get_highest_checkpoint_at_or_before(&mut conn, 1, 2000).await?;
    assert!(result.is_some());
    let stored = result.unwrap();
    assert_eq!(stored.header_number, BigDecimal::from(2000));
    assert_eq!(stored.digest, format!("0x{:x}", checkpoint2.digest));

    // Test get_lowest_checkpoint_at_or_after with exact match
    // Querying at 2000 should return checkpoint2 (inclusive)
    let result = continuity_blocks::get_lowest_checkpoint_at_or_after(&mut conn, 1, 2000).await?;
    assert!(result.is_some());
    let stored = result.unwrap();
    assert_eq!(stored.header_number, BigDecimal::from(2000));
    assert_eq!(stored.digest, format!("0x{:x}", checkpoint2.digest));

    // Test boundary cases: at_or_before at exactly the lowest value
    let result = continuity_blocks::get_highest_checkpoint_at_or_before(&mut conn, 1, 1000).await?;
    assert!(result.is_some());
    let stored = result.unwrap();
    assert_eq!(stored.header_number, BigDecimal::from(1000));
    assert_eq!(stored.digest, format!("0x{:x}", checkpoint1.digest));

    // Test boundary cases: at_or_after at exactly the highest value
    let result = continuity_blocks::get_lowest_checkpoint_at_or_after(&mut conn, 1, 3000).await?;
    assert!(result.is_some());
    let stored = result.unwrap();
    assert_eq!(stored.header_number, BigDecimal::from(3000));
    assert_eq!(stored.digest, format!("0x{:x}", checkpoint3.digest));

    Ok(())
}

#[cfg_attr(not(feature = "integration-tests"), ignore)]
#[tokio::test]
async fn test_multi_type_same_header() -> Result<()> {
    let (_container, mut conn) = setup_test_db().await?;

    // Create attestation and regular block at same header_number
    let attestation = ContinuityBlockItem::from_attestation(
        1,
        1000,
        "0x1111111111111111111111111111111111111111111111111111111111111111".to_string(),
    );

    let regular = ContinuityBlockItem::from_regular_block(
        1,
        1000,
        "0x2222222222222222222222222222222222222222222222222222222222222222".to_string(),
    );

    // Both inserts should succeed (different partial indexes)
    continuity_blocks::insert_attestation(&mut conn, &attestation).await?;
    continuity_blocks::insert_continuity_blocks(&mut conn, vec![regular]).await?;

    // Query for attestation should return attestation
    let result =
        continuity_blocks::get_highest_attestation_at_or_before(&mut conn, 1, 1000).await?;
    assert!(result.is_some());
    let found = result.unwrap();
    assert!(found.is_attestation());
    assert!(!found.is_checkpoint());
    assert_eq!(
        found.digest,
        "0x1111111111111111111111111111111111111111111111111111111111111111"
    );

    Ok(())
}

#[cfg_attr(not(feature = "integration-tests"), ignore)]
#[tokio::test]
async fn test_get_continuity_blocks_in_range() -> Result<()> {
    let (_container, mut conn) = setup_test_db().await?;

    // Insert mixed types
    let attestation = ContinuityBlockItem::from_attestation(
        1,
        1000,
        "0x1000111111111111111111111111111111111111111111111111111111111111".to_string(),
    );

    let checkpoint = ContinuityBlockItem::from_checkpoint(
        1,
        &AttestationCheckpoint {
            block_number: 1005,
            digest: H256::from_low_u64_be(1005),
        },
    );

    let regular1 = ContinuityBlockItem::from_regular_block(
        1,
        1002,
        "0x1002222222222222222222222222222222222222222222222222222222222222".to_string(),
    );

    let regular2 = ContinuityBlockItem::from_regular_block(
        1,
        1003,
        "0x1003222222222222222222222222222222222222222222222222222222222222".to_string(),
    );

    // Also insert a duplicate at header 1002 with different type
    let attestation_duplicate = ContinuityBlockItem::from_attestation(
        1,
        1002,
        "0x1002AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
    );

    continuity_blocks::insert_attestation(&mut conn, &attestation).await?;
    continuity_blocks::insert_checkpoint(&mut conn, &checkpoint).await?;
    continuity_blocks::insert_continuity_blocks(&mut conn, vec![regular1, regular2]).await?;
    continuity_blocks::insert_attestation(&mut conn, &attestation_duplicate).await?;

    // Get range from 1000 to 1005
    let blocks =
        continuity_blocks::get_continuity_blocks_in_range(&mut conn, 1, 1000, 1005).await?;

    // Should get 4 blocks (deduplicated by header_number, 1002 appears twice but we get first)
    assert_eq!(blocks.len(), 4);

    // Verify ordering
    assert_eq!(blocks[0].header_number, BigDecimal::from(1000));
    assert_eq!(blocks[1].header_number, BigDecimal::from(1002));
    assert_eq!(blocks[2].header_number, BigDecimal::from(1003));
    assert_eq!(blocks[3].header_number, BigDecimal::from(1005));

    // Verify deduplication - should get first entry for 1002 (regular block by id order)
    assert!(!blocks[1].is_attestation());
    assert!(!blocks[1].is_checkpoint());

    Ok(())
}
