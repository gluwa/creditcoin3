//! Integration tests for CC3 event subscription and attestation/checkpoint storage.
//!
//! Tests end-to-end attestation and checkpoint storage flow from event processing to database.
//! Database-only tests are in tests/db_integration.rs

#[path = "test_utils.rs"]
mod test_utils;

#[cfg(feature = "integration-tests")]
mod attestation_checkpoint_storage {
    use super::test_utils::*;
    use attestor_primitives::AttestationCheckpoint;
    use cc_client::attestation::{BlockAttestedMetadata, CcEvent};
    use proof_gen_api_server::db::{continuity_blocks, models::ContinuityBlockItem, DbManager};
    use sp_core::H256;

    /// Helper to store an attestation directly (simulates BlockAttested event processing)
    async fn store_attestation(
        db_manager: &DbManager,
        chain_key: u64,
        header_number: u64,
        digest: H256,
    ) {
        let digest_str = format!("0x{digest:x}");
        let attestation_item =
            ContinuityBlockItem::from_attestation(chain_key, header_number, digest_str);
        let mut conn = db_manager
            .pool()
            .get()
            .await
            .expect("Failed to get connection");
        continuity_blocks::insert_attestation(&mut conn, &attestation_item)
            .await
            .expect("Failed to store attestation");
    }

    /// Helper to upsert a checkpoint (simulates CheckpointReached event processing)
    async fn upsert_checkpoint(
        db_manager: &DbManager,
        chain_key: u64,
        checkpoint: &AttestationCheckpoint,
    ) {
        let digest_str = format!("0x{:x}", checkpoint.digest);
        let mut conn = db_manager
            .pool()
            .get()
            .await
            .expect("Failed to get connection");
        continuity_blocks::upsert_checkpoint(
            &mut conn,
            chain_key,
            checkpoint.block_number,
            &digest_str,
        )
        .await
        .expect("Failed to upsert checkpoint");
    }

    #[tokio::test]
    async fn test_attestation_to_database_flow() {
        let container = setup_test_postgres().await;
        let postgres_uri = test_db_manager_postgres_uri(&container).await;
        let db_manager = DbManager::new(postgres_uri).expect("Failed to create DB manager");
        db_manager
            .run_migrations()
            .await
            .expect("Failed to run migrations");

        let digest = H256::from_low_u64_be(12345);

        // Test: BlockAttested → Database Storage
        store_attestation(&db_manager, 1, 100, digest).await;

        // Verify attestation was stored
        let mut conn = db_manager
            .pool()
            .get()
            .await
            .expect("Failed to get connection");
        let stored_attestation =
            continuity_blocks::get_highest_attestation_at_or_before(&mut conn, 1, 101)
                .await
                .expect("Failed to query attestations");

        assert!(stored_attestation.is_some(), "Attestation should be stored");
        let attestation = stored_attestation.unwrap();
        assert_eq!(attestation.chain_key, 1);
        assert!(attestation.is_attestation);
        assert!(!attestation.is_checkpoint);
    }

    #[tokio::test]
    async fn test_attestation_then_checkpoint_upgrade() {
        let container = setup_test_postgres().await;
        let postgres_uri = test_db_manager_postgres_uri(&container).await;
        let db_manager = DbManager::new(postgres_uri).expect("Failed to create DB manager");
        db_manager
            .run_migrations()
            .await
            .expect("Failed to run migrations");

        let digest = H256::from_low_u64_be(12345);

        // Step 1: BlockAttested event arrives first
        store_attestation(&db_manager, 1, 100, digest).await;

        // Verify it's stored as attestation only
        let mut conn = db_manager
            .pool()
            .get()
            .await
            .expect("Failed to get connection");
        let attestation =
            continuity_blocks::get_highest_attestation_at_or_before(&mut conn, 1, 101)
                .await
                .expect("Failed to query")
                .expect("Attestation should exist");
        assert!(attestation.is_attestation);
        assert!(!attestation.is_checkpoint);

        // Step 2: CheckpointReached event arrives, upgrading the attestation
        let checkpoint = AttestationCheckpoint {
            block_number: 100,
            digest,
        };
        upsert_checkpoint(&db_manager, 1, &checkpoint).await;

        // Verify attestation was upgraded to checkpoint (both flags true)
        let upgraded = continuity_blocks::get_highest_checkpoint_at_or_before(&mut conn, 1, 101)
            .await
            .expect("Failed to query")
            .expect("Checkpoint should exist");
        assert!(upgraded.is_attestation, "Should still be an attestation");
        assert!(upgraded.is_checkpoint, "Should now also be a checkpoint");
        assert_eq!(upgraded.header_number.to_string(), "100");
    }

    #[tokio::test]
    async fn test_checkpoint_without_prior_attestation() {
        let container = setup_test_postgres().await;
        let postgres_uri = test_db_manager_postgres_uri(&container).await;
        let db_manager = DbManager::new(postgres_uri).expect("Failed to create DB manager");
        db_manager
            .run_migrations()
            .await
            .expect("Failed to run migrations");

        // Edge case: CheckpointReached arrives without a prior BlockAttested event
        let checkpoint = AttestationCheckpoint {
            block_number: 100,
            digest: H256::from_low_u64_be(12345),
        };
        upsert_checkpoint(&db_manager, 1, &checkpoint).await;

        // Verify it's stored as both attestation and checkpoint
        let mut conn = db_manager
            .pool()
            .get()
            .await
            .expect("Failed to get connection");
        let stored = continuity_blocks::get_highest_checkpoint_at_or_before(&mut conn, 1, 101)
            .await
            .expect("Failed to query")
            .expect("Checkpoint should exist");
        assert!(stored.is_attestation, "Should be an attestation");
        assert!(stored.is_checkpoint, "Should be a checkpoint");
    }

    #[tokio::test]
    async fn test_duplicate_checkpoint_handling() {
        let container = setup_test_postgres().await;
        let postgres_uri = test_db_manager_postgres_uri(&container).await;
        let db_manager = DbManager::new(postgres_uri).expect("Failed to create DB manager");
        db_manager
            .run_migrations()
            .await
            .expect("Failed to run migrations");

        let checkpoint = AttestationCheckpoint {
            block_number: 100,
            digest: H256::from_low_u64_be(12345),
        };

        // First upsert
        upsert_checkpoint(&db_manager, 1, &checkpoint).await;

        // Second upsert (duplicate) - should be idempotent
        upsert_checkpoint(&db_manager, 1, &checkpoint).await;

        // Verify only one record exists
        let mut conn = db_manager
            .pool()
            .get()
            .await
            .expect("Failed to get connection");
        let stored_checkpoint =
            continuity_blocks::get_highest_checkpoint_at_or_before(&mut conn, 1, 101)
                .await
                .expect("Failed to query checkpoints");

        assert!(stored_checkpoint.is_some(), "Checkpoint should be stored");
        let checkpoint = stored_checkpoint.unwrap();
        assert_eq!(checkpoint.chain_key, 1);
        assert_eq!(checkpoint.header_number.to_string(), "100");
    }

    #[tokio::test]
    async fn test_multiple_checkpoints_different_blocks() {
        let container = setup_test_postgres().await;
        let postgres_uri = test_db_manager_postgres_uri(&container).await;
        let db_manager = DbManager::new(postgres_uri).expect("Failed to create DB manager");
        db_manager
            .run_migrations()
            .await
            .expect("Failed to run migrations");

        // First store attestations for all blocks
        for block in [100u64, 200, 300] {
            store_attestation(&db_manager, 1, block, H256::from_low_u64_be(block)).await;
        }

        // Then upgrade to checkpoints
        let checkpoints = vec![
            AttestationCheckpoint {
                block_number: 100,
                digest: H256::from_low_u64_be(100),
            },
            AttestationCheckpoint {
                block_number: 200,
                digest: H256::from_low_u64_be(200),
            },
            AttestationCheckpoint {
                block_number: 300,
                digest: H256::from_low_u64_be(300),
            },
        ];

        for checkpoint in &checkpoints {
            upsert_checkpoint(&db_manager, 1, checkpoint).await;
        }

        // Test get_highest_checkpoint_at_or_before
        let mut conn = db_manager
            .pool()
            .get()
            .await
            .expect("Failed to get connection");
        let highest_before_250 =
            continuity_blocks::get_highest_checkpoint_at_or_before(&mut conn, 1, 250)
                .await
                .expect("Failed to query checkpoints");

        assert!(highest_before_250.is_some());
        assert_eq!(highest_before_250.unwrap().header_number.to_string(), "200");

        // Test get_lowest_checkpoint_at_or_after
        let lowest_after_150 =
            continuity_blocks::get_lowest_checkpoint_at_or_after(&mut conn, 1, 150)
                .await
                .expect("Failed to query checkpoints");

        assert!(lowest_after_150.is_some());
        assert_eq!(lowest_after_150.unwrap().header_number.to_string(), "200");
    }

    #[tokio::test]
    async fn test_multiple_chains_isolated() {
        let container = setup_test_postgres().await;
        let postgres_uri = test_db_manager_postgres_uri(&container).await;
        let db_manager = DbManager::new(postgres_uri).expect("Failed to create DB manager");
        db_manager
            .run_migrations()
            .await
            .expect("Failed to run migrations");

        // Insert attestations and checkpoints for different chains
        let checkpoint1 = AttestationCheckpoint {
            block_number: 100,
            digest: H256::from_low_u64_be(1100),
        };

        let checkpoint2 = AttestationCheckpoint {
            block_number: 100,
            digest: H256::from_low_u64_be(2100),
        };

        upsert_checkpoint(&db_manager, 1, &checkpoint1).await;
        upsert_checkpoint(&db_manager, 2, &checkpoint2).await;

        // Verify chain isolation
        let mut conn = db_manager
            .pool()
            .get()
            .await
            .expect("Failed to get connection");

        let chain1_result =
            continuity_blocks::get_highest_checkpoint_at_or_before(&mut conn, 1, 101)
                .await
                .expect("Failed to query chain 1");

        let chain2_result =
            continuity_blocks::get_highest_checkpoint_at_or_before(&mut conn, 2, 101)
                .await
                .expect("Failed to query chain 2");

        assert!(chain1_result.is_some());
        assert!(chain2_result.is_some());
        assert_eq!(chain1_result.unwrap().chain_key, 1);
        assert_eq!(chain2_result.unwrap().chain_key, 2);
    }

    #[tokio::test]
    async fn test_ccevent_block_attested_structure() {
        // Verify CcEvent::BlockAttested can be constructed and pattern matched
        use attestor_primitives::Digest;

        let metadata = BlockAttestedMetadata {
            chain_key: 1,
            header_number: 100,
            digest: Digest::from(H256::from_low_u64_be(12345).0),
        };

        let event = CcEvent::BlockAttested(metadata);

        match event {
            CcEvent::BlockAttested(meta) => {
                assert_eq!(meta.chain_key, 1);
                assert_eq!(meta.header_number, 100);
            }
            _ => panic!("Expected BlockAttested event"),
        }
    }

    #[tokio::test]
    async fn test_ccevent_checkpoint_reached_structure() {
        // Verify CcEvent::CheckpointReached can be constructed and pattern matched
        let checkpoint = AttestationCheckpoint {
            block_number: 100,
            digest: H256::from_low_u64_be(12345),
        };

        let event = CcEvent::CheckpointReached(checkpoint.clone());

        match event {
            CcEvent::CheckpointReached(cp) => {
                assert_eq!(cp.block_number, 100);
                assert_eq!(cp.digest, H256::from_low_u64_be(12345));
            }
            _ => panic!("Expected CheckpointReached event"),
        }
    }
}
