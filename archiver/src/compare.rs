//! `compare` subcommand — diff merkle roots between two sled databases.
//!
//! Both databases are walked in lockstep over the requested height range, so
//! comparing very large spans does not buffer either database in memory.

use std::cmp::Ordering;

use anyhow::{Context, Result};

use crate::config::CompareArgs;
use crate::store::RootStore;

/// Compare the two databases described by `args`.
///
/// Returns the number of differing heights (root mismatches plus heights
/// present in only one database). A return value of `0` means the two
/// databases agree on every height in the range.
pub fn run_compare(args: &CompareArgs) -> Result<u64> {
    let db_a = RootStore::open(&args.db_a)
        .with_context(|| format!("failed to open db-a at {:?}", args.db_a))?;
    let db_b = RootStore::open(&args.db_b)
        .with_context(|| format!("failed to open db-b at {:?}", args.db_b))?;

    let latest_a = db_a.latest_height()?;
    let latest_b = db_b.latest_height()?;

    let from = args.from;
    let to = match args.to {
        Some(to) => to,
        // Default to the highest height present in *both* databases.
        None => match (latest_a, latest_b) {
            (Some(a), Some(b)) => a.min(b),
            _ => {
                println!("one or both databases are empty; nothing to compare");
                return Ok(0);
            }
        },
    };

    anyhow::ensure!(from <= to, "invalid range: from ({from}) > to ({to})");

    println!(
        "comparing roots in [{from}, {to}]  (db-a latest={}, db-b latest={})",
        fmt_height(latest_a),
        fmt_height(latest_b),
    );

    let mut ia = db_a.iter_range(from, to);
    let mut ib = db_b.iter_range(from, to);

    let mut ca = ia.next().transpose()?;
    let mut cb = ib.next().transpose()?;

    let mut mismatches: u64 = 0;
    let mut compared: u64 = 0;

    loop {
        match (ca, cb) {
            (None, None) => break,
            (Some((ha, ra)), Some((hb, rb))) => match ha.cmp(&hb) {
                Ordering::Equal => {
                    compared += 1;
                    if ra != rb {
                        mismatches += 1;
                        println!("MISMATCH @ {ha}: a={ra:#x} b={rb:#x}");
                    }
                    ca = ia.next().transpose()?;
                    cb = ib.next().transpose()?;
                }
                Ordering::Less => {
                    mismatches += 1;
                    println!("MISSING in db-b @ {ha}");
                    ca = ia.next().transpose()?;
                }
                Ordering::Greater => {
                    mismatches += 1;
                    println!("MISSING in db-a @ {hb}");
                    cb = ib.next().transpose()?;
                }
            },
            (Some((ha, _)), None) => {
                mismatches += 1;
                println!("MISSING in db-b @ {ha}");
                ca = ia.next().transpose()?;
            }
            (None, Some((hb, _))) => {
                mismatches += 1;
                println!("MISSING in db-a @ {hb}");
                cb = ib.next().transpose()?;
            }
        }
    }

    if mismatches == 0 {
        println!("✓ {compared} roots match in [{from}, {to}]");
    } else {
        println!("✗ {mismatches} mismatches in [{from}, {to}] ({compared} heights compared)");
    }

    Ok(mismatches)
}

fn fmt_height(h: Option<u64>) -> String {
    match h {
        Some(h) => h.to_string(),
        None => "none".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sp_core::H256;

    fn args(db_a: std::path::PathBuf, db_b: std::path::PathBuf, to: Option<u64>) -> CompareArgs {
        CompareArgs {
            db_a,
            db_b,
            from: 0,
            to,
        }
    }

    #[test]
    fn identical_databases_have_no_mismatches() {
        let dir = tempfile::tempdir().unwrap();
        let pa = dir.path().join("a.sled");
        let pb = dir.path().join("b.sled");

        let entries: Vec<(u64, H256)> = (0..5).map(|i| (i, H256::random())).collect();
        {
            let a = RootStore::open(&pa).unwrap();
            let b = RootStore::open(&pb).unwrap();
            a.put_roots(&entries).unwrap();
            b.put_roots(&entries).unwrap();
        }

        assert_eq!(run_compare(&args(pa, pb, Some(4))).unwrap(), 0);
    }

    #[test]
    fn detects_mismatch_and_missing_heights() {
        let dir = tempfile::tempdir().unwrap();
        let pa = dir.path().join("a.sled");
        let pb = dir.path().join("b.sled");

        let shared: Vec<(u64, H256)> = (0..5).map(|i| (i, H256::random())).collect();
        {
            let a = RootStore::open(&pa).unwrap();
            let b = RootStore::open(&pb).unwrap();
            a.put_roots(&shared).unwrap();
            b.put_roots(&shared).unwrap();

            // Height 5 has different roots in each database.
            a.put_roots(&[(5, H256::random())]).unwrap();
            b.put_roots(&[(5, H256::random())]).unwrap();

            // Height 6 exists only in db-a.
            a.put_roots(&[(6, H256::random())]).unwrap();
        }

        // One root mismatch (height 5) + one missing height (6) = 2.
        assert_eq!(run_compare(&args(pa, pb, Some(6))).unwrap(), 2);
    }

    #[test]
    fn empty_database_compares_cleanly_with_default_range() {
        let dir = tempfile::tempdir().unwrap();
        let pa = dir.path().join("a.sled");
        let pb = dir.path().join("b.sled");

        {
            let a = RootStore::open(&pa).unwrap();
            a.put_roots(&[(0, H256::random())]).unwrap();
            // db-b left empty.
            let _b = RootStore::open(&pb).unwrap();
        }

        // No explicit `to`, and db-b is empty → nothing to compare.
        assert_eq!(run_compare(&args(pa, pb, None)).unwrap(), 0);
    }
}
