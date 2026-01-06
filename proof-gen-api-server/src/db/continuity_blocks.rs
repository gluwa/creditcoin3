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

/// Insert an attestation using partial unique index with filter_target.
/// The partial unique index `continuity_blocks_attestations` ensures no duplicate
/// attestations at the same (chain_key, header_number) WHERE is_attestation=true AND is_checkpoint=false.
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

    diesel::insert_into(continuity_blocks::table)
        .values(attestation)
        .on_conflict((
            continuity_blocks::chain_key,
            continuity_blocks::header_number,
        ))
        .filter_target(sql::<Bool>(
            "is_attestation = TRUE AND is_checkpoint = FALSE",
        ))
        .do_nothing()
        .execute(conn)
        .await?;

    Ok(())
}

/// Insert a checkpoint using partial unique index with filter_target.
/// The partial unique index `continuity_blocks_checkpoints` ensures no duplicate
/// checkpoints at the same (chain_key, header_number) WHERE is_attestation=false AND is_checkpoint=true.
pub async fn insert_checkpoint(
    conn: &mut AsyncPgConnection,
    checkpoint: &ContinuityBlockItem,
) -> Result<()> {
    use diesel::dsl::sql;
    use diesel::sql_types::Bool;

    // Validate that the input is actually a checkpoint
    anyhow::ensure!(
        !checkpoint.is_attestation && checkpoint.is_checkpoint,
        "insert_checkpoint requires is_attestation=false and is_checkpoint=true, got is_attestation={}, is_checkpoint={}",
        checkpoint.is_attestation,
        checkpoint.is_checkpoint
    );

    diesel::insert_into(continuity_blocks::table)
        .values(checkpoint)
        .on_conflict((
            continuity_blocks::chain_key,
            continuity_blocks::header_number,
        ))
        .filter_target(sql::<Bool>(
            "is_attestation = FALSE AND is_checkpoint = TRUE",
        ))
        .do_nothing()
        .execute(conn)
        .await?;

    Ok(())
}

/// Batch insert continuity blocks using partial unique indexes with filter_target.
/// The partial unique indexes ensure no duplicate blocks of the same type at the same
/// (chain_key, header_number). Since blocks can have different types, we insert them
/// individually to apply the correct conflict resolution for each type.
pub async fn insert_continuity_blocks(
    conn: &mut AsyncPgConnection,
    blocks: Vec<ContinuityBlockItem>,
) -> Result<()> {
    use diesel::dsl::sql;
    use diesel::sql_types::Bool;

    for block in blocks {
        match (block.is_attestation, block.is_checkpoint) {
            // Attestation: (true, false)
            (true, false) => {
                diesel::insert_into(continuity_blocks::table)
                    .values(&block)
                    .on_conflict((
                        continuity_blocks::chain_key,
                        continuity_blocks::header_number,
                    ))
                    .filter_target(sql::<Bool>(
                        "is_attestation = TRUE AND is_checkpoint = FALSE",
                    ))
                    .do_nothing()
                    .execute(conn)
                    .await?;
            }
            // Checkpoint: (false, true)
            (false, true) => {
                diesel::insert_into(continuity_blocks::table)
                    .values(&block)
                    .on_conflict((
                        continuity_blocks::chain_key,
                        continuity_blocks::header_number,
                    ))
                    .filter_target(sql::<Bool>(
                        "is_attestation = FALSE AND is_checkpoint = TRUE",
                    ))
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
            // Invalid state (both true)
            _ => anyhow::bail!(
                "Invalid block flags: is_attestation={}, is_checkpoint={}. Both cannot be true.",
                block.is_attestation,
                block.is_checkpoint
            ),
        }
    }

    Ok(())
}
