//! Database operations for unified continuity blocks storage.
//!
//! This module handles attestations, checkpoints, and regular blocks in a single table,
//! using boolean flags to distinguish between types.

use anyhow::Result;
use bigdecimal::BigDecimal;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};

use crate::db::models::ContinuityBlockItem;
use crate::db::schema::continuity_blocks;

/// Get highest attestation at or before the given header number (inclusive)
pub async fn get_highest_attestation_at_or_before(
    conn: &mut AsyncPgConnection,
    chain_key: u64,
    header_number: u64,
) -> Result<Option<ContinuityBlockItem>> {
    let header_num = BigDecimal::from(header_number);

    let result = continuity_blocks::table
        .filter(continuity_blocks::chain_key.eq(chain_key as i64))
        .filter(continuity_blocks::header_number.le(&header_num))
        .filter(continuity_blocks::is_attestation.eq(true))
        .order(continuity_blocks::header_number.desc())
        .first::<ContinuityBlockItem>(conn)
        .await
        .optional()?;

    Ok(result)
}

/// Get lowest attestation at or after the given header number (inclusive)
pub async fn get_lowest_attestation_at_or_after(
    conn: &mut AsyncPgConnection,
    chain_key: u64,
    header_number: u64,
) -> Result<Option<ContinuityBlockItem>> {
    let header_num = BigDecimal::from(header_number);

    let result = continuity_blocks::table
        .filter(continuity_blocks::chain_key.eq(chain_key as i64))
        .filter(continuity_blocks::header_number.ge(&header_num))
        .filter(continuity_blocks::is_attestation.eq(true))
        .order(continuity_blocks::header_number.asc())
        .first::<ContinuityBlockItem>(conn)
        .await
        .optional()?;

    Ok(result)
}

/// Get highest checkpoint at or before the given header number (inclusive)
pub async fn get_highest_checkpoint_at_or_before(
    conn: &mut AsyncPgConnection,
    chain_key: u64,
    header_number: u64,
) -> Result<Option<ContinuityBlockItem>> {
    let header_num = BigDecimal::from(header_number);

    let result = continuity_blocks::table
        .filter(continuity_blocks::chain_key.eq(chain_key as i64))
        .filter(continuity_blocks::header_number.le(&header_num))
        .filter(continuity_blocks::is_checkpoint.eq(true))
        .order(continuity_blocks::header_number.desc())
        .first::<ContinuityBlockItem>(conn)
        .await
        .optional()?;

    Ok(result)
}

/// Get lowest checkpoint at or after the given header number (inclusive)
pub async fn get_lowest_checkpoint_at_or_after(
    conn: &mut AsyncPgConnection,
    chain_key: u64,
    header_number: u64,
) -> Result<Option<ContinuityBlockItem>> {
    let header_num = BigDecimal::from(header_number);

    let result = continuity_blocks::table
        .filter(continuity_blocks::chain_key.eq(chain_key as i64))
        .filter(continuity_blocks::header_number.ge(&header_num))
        .filter(continuity_blocks::is_checkpoint.eq(true))
        .order(continuity_blocks::header_number.asc())
        .first::<ContinuityBlockItem>(conn)
        .await
        .optional()?;

    Ok(result)
}

/// Get all continuity blocks in the given range, deduplicated by header_number.
/// When multiple blocks exist at the same header_number (different types),
/// returns the first one ordered by id.
pub async fn get_continuity_blocks_in_range(
    conn: &mut AsyncPgConnection,
    chain_key: u64,
    first: u64,
    last: u64,
) -> Result<Vec<ContinuityBlockItem>> {
    let first_num = BigDecimal::from(first);
    let last_num = BigDecimal::from(last);

    let blocks = continuity_blocks::table
        .filter(continuity_blocks::chain_key.eq(chain_key as i64))
        .filter(continuity_blocks::header_number.ge(&first_num))
        .filter(continuity_blocks::header_number.le(&last_num))
        .order_by((
            continuity_blocks::header_number.asc(),
            continuity_blocks::id.asc(),
        ))
        .distinct_on(continuity_blocks::header_number)
        .load::<ContinuityBlockItem>(conn)
        .await?;

    Ok(blocks)
}

/// Insert an attestation, skipping if any attestation (or checkpoint) already exists.
///
/// This handles the race condition where CheckpointReached arrives before BlockAttested:
/// - If a checkpoint exists (is_attestation=TRUE, is_checkpoint=TRUE), we skip insertion
/// - If an attestation exists (is_attestation=TRUE, is_checkpoint=FALSE), we skip insertion
/// - Only insert if no attestation exists at this (chain_key, header_number)
pub async fn insert_attestation(
    conn: &mut AsyncPgConnection,
    attestation: &ContinuityBlockItem,
) -> Result<()> {
    use diesel::dsl::sql;
    use diesel::sql_types::Bool;

    // Validate that the input is actually an attestation
    anyhow::ensure!(
        attestation.is_attestation && !attestation.is_checkpoint,
        "insert_attestation requires is_attestation=true and is_checkpoint=false, got is_attestation={}, is_checkpoint={}",
        attestation.is_attestation,
        attestation.is_checkpoint
    );

    // Use filter_target with is_attestation=TRUE to match BOTH attestation-only rows
    // and checkpoint rows (which are also attestations). This prevents inserting a
    // duplicate attestation if a checkpoint already exists at this location.
    diesel::insert_into(continuity_blocks::table)
        .values(attestation)
        .on_conflict((
            continuity_blocks::chain_key,
            continuity_blocks::header_number,
        ))
        .filter_target(sql::<Bool>("is_attestation = TRUE"))
        .do_nothing()
        .execute(conn)
        .await?;

    Ok(())
}

/// Mark an existing attestation as a checkpoint by updating is_checkpoint to true.
/// If no attestation exists at this location, inserts a new row with both flags true.
/// A checkpoint is always also an attestation (is_attestation=true, is_checkpoint=true).
///
/// This function handles the case where:
/// 1. BlockAttested event comes first → creates row with (true, false)
/// 2. CheckpointReached event comes later → updates row to (true, true)
///
/// If the checkpoint arrives before the attestation (edge case), it inserts directly.
///
/// Uses INSERT ... ON CONFLICT DO UPDATE to atomically handle all cases in a single statement.
pub async fn upsert_checkpoint(
    conn: &mut AsyncPgConnection,
    chain_key: u64,
    header_number: u64,
    digest: &str,
) -> Result<()> {
    use diesel::sql_types::{BigInt, Numeric, Text};

    let header_num = BigDecimal::from(header_number);

    // Use ON CONFLICT DO UPDATE with WHERE clause:
    // - If attestation exists (true, false): update is_checkpoint to true
    // - If checkpoint exists (true, true): no-op (WHERE clause prevents update)
    // - If nothing exists: insert new checkpoint
    diesel::sql_query(
        "INSERT INTO continuity_blocks (chain_key, header_number, digest, is_attestation, is_checkpoint)
         VALUES ($1, $2, $3, TRUE, TRUE)
         ON CONFLICT (chain_key, header_number)
         WHERE is_attestation = TRUE
         DO UPDATE SET is_checkpoint = TRUE
         WHERE continuity_blocks.is_checkpoint = FALSE",
    )
    .bind::<BigInt, _>(chain_key as i64)
    .bind::<Numeric, _>(&header_num)
    .bind::<Text, _>(digest)
    .execute(conn)
    .await?;

    Ok(())
}

/// Batch insert continuity blocks using partial unique indexes with filter_target.
/// For attestations (including checkpoints), uses a unified index on is_attestation=TRUE
/// to prevent duplicates. For regular blocks, uses a separate index on is_attestation=FALSE.
/// This handles the race condition where events arrive out-of-order.
pub async fn insert_continuity_blocks(
    conn: &mut AsyncPgConnection,
    blocks: Vec<ContinuityBlockItem>,
) -> Result<()> {
    use diesel::dsl::sql;
    use diesel::sql_types::Bool;

    for block in blocks {
        match (block.is_attestation, block.is_checkpoint) {
            // Attestation only: (true, false)
            // Use is_attestation=TRUE to skip if ANY attestation exists (including checkpoints)
            // This handles the race where CheckpointReached arrives before BlockAttested
            (true, false) => {
                diesel::insert_into(continuity_blocks::table)
                    .values(&block)
                    .on_conflict((
                        continuity_blocks::chain_key,
                        continuity_blocks::header_number,
                    ))
                    .filter_target(sql::<Bool>("is_attestation = TRUE"))
                    .do_nothing()
                    .execute(conn)
                    .await?;
            }
            // Checkpoint (which is also an attestation): (true, true)
            // Use is_attestation=TRUE to skip if ANY attestation exists (unified index)
            (true, true) => {
                diesel::insert_into(continuity_blocks::table)
                    .values(&block)
                    .on_conflict((
                        continuity_blocks::chain_key,
                        continuity_blocks::header_number,
                    ))
                    .filter_target(sql::<Bool>("is_attestation = TRUE"))
                    .do_nothing()
                    .execute(conn)
                    .await?;
            }
            // Regular: (false, false)
            (false, false) => {
                diesel::insert_into(continuity_blocks::table)
                    .values(&block)
                    .on_conflict((
                        continuity_blocks::chain_key,
                        continuity_blocks::header_number,
                    ))
                    .filter_target(sql::<Bool>(
                        "is_attestation = FALSE AND is_checkpoint = FALSE",
                    ))
                    .do_nothing()
                    .execute(conn)
                    .await?;
            }
            // Invalid state: checkpoint without attestation (false, true)
            (false, true) => anyhow::bail!(
                "Invalid block flags: is_attestation={}, is_checkpoint={}. A checkpoint must also be an attestation.",
                block.is_attestation,
                block.is_checkpoint
            ),
        }
    }

    Ok(())
}
