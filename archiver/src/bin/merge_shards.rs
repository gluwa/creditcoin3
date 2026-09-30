//! Merge several archiver sled databases ("shards", e.g. from `--end-height` range runs) into
//! one database in the same format, so a single tip-following archiver can start from the
//! full history.
//!
//! Reads and writes go through the archiver's own [`RootStore`], so the output has the same
//! key/value layout, entry counter and chain-id pin as a database the archiver wrote itself.
//! Inputs are opened but not written to (beyond sled's own open bookkeeping, and the entry
//! counter the store records on open if a database lacks one).
//!
//! Checks, all fatal:
//! - no input is given twice, and none holds trees or meta keys the store does not write
//!   (they would not be carried over);
//! - every shard pins the same source chain id (or `--chain-id`, when given);
//! - every shard is gap-free across its own `[first, last]`;
//! - the shards leave no gap between each other;
//! - where shards overlap, the overlapping heights hold identical root and block hash
//!   (enforced by the store's reorg guard as they are written);
//! - re-reading every shard afterwards, the output holds the same root, and the same block
//!   hash wherever the shard has one, at every height;
//! - the output is gap-free over the whole span and its entry count matches.

use std::collections::HashSet;
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{bail, ensure, Context, Result};
use clap::Parser;

// Shared with the archiver binary; this tool only needs part of it.
#[allow(dead_code)]
#[path = "../store.rs"]
mod store;

use store::RootStore;

/// Log copy progress every this many blocks.
const PROGRESS_EVERY: u64 = 1_000_000;

#[derive(Parser, Debug)]
#[command(
    name = "merge-shards",
    about = "Merge contiguous archiver sled databases into one database in the same format"
)]
struct Config {
    /// Input shard database directories, in any order. Repeat the flag or comma-separate.
    #[arg(long = "input", required = true, value_delimiter = ',')]
    inputs: Vec<PathBuf>,

    /// Output database directory. Must not exist yet.
    #[arg(long)]
    output: PathBuf,

    /// Expected source chain id. Required only if some input was never pinned; otherwise
    /// every pinned input must match it.
    #[arg(long)]
    chain_id: Option<u64>,

    /// Blocks read and written per batch.
    #[arg(long, default_value = "100000")]
    batch_size: NonZeroU64,

    /// Run the input checks and print the plan without creating the output.
    #[arg(long)]
    dry_run: bool,
}

/// An inclusive height range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Span {
    first: u64,
    last: u64,
}

impl Span {
    fn len(&self) -> u64 {
        self.last - self.first + 1
    }
}

struct Shard {
    path: PathBuf,
    store: RootStore,
    span: Span,
}

/// Split `span` into consecutive inclusive batches of at most `size` heights.
fn chunks(span: Span, size: NonZeroU64) -> impl Iterator<Item = Span> {
    let mut next = Some(span.first);
    std::iter::from_fn(move || {
        let first = next?;
        let last = first.saturating_add(size.get() - 1).min(span.last);
        next = (last < span.last).then(|| last + 1);
        Some(Span { first, last })
    })
}

/// How spans (sorted by `first`) join up: the heights covered by none of them, and the
/// heights a span shares with the ones before it. Both are inclusive ranges.
#[derive(Debug, Default, PartialEq, Eq)]
struct Joins {
    gaps: Vec<Span>,
    overlaps: Vec<Span>,
}

fn joins(sorted: &[Span]) -> Joins {
    let mut out = Joins::default();
    let Some((head, rest)) = sorted.split_first() else {
        return out;
    };
    let mut covered_through = head.last;
    for span in rest {
        if span.first > covered_through.saturating_add(1) {
            out.gaps.push(Span {
                first: covered_through + 1,
                last: span.first - 1,
            });
        } else if span.first <= covered_through {
            out.overlaps.push(Span {
                first: span.first,
                last: span.last.min(covered_through),
            });
        }
        covered_through = covered_through.max(span.last);
    }
    out
}

/// The chain id the output is pinned to: `expected` if given, else the one all inputs share.
fn resolve_chain_id(pins: &[(PathBuf, Option<u64>)], expected: Option<u64>) -> Result<u64> {
    let mut resolved = expected;
    for (path, pin) in pins {
        match (pin, resolved) {
            (Some(id), Some(want)) if *id != want => {
                bail!(
                    "{} is pinned to chain id {id}, expected {want}",
                    path.display()
                )
            }
            (Some(id), None) => resolved = Some(*id),
            (None, None) => bail!(
                "{} has no chain id pinned; pass --chain-id to set the output's",
                path.display()
            ),
            _ => {}
        }
    }
    resolved.context("no inputs")
}

fn open_shard(path: PathBuf) -> Result<(Shard, Option<u64>)> {
    ensure!(path.is_dir(), "{} is not a directory", path.display());
    let store = RootStore::open(&path)?;
    let foreign = store.foreign_contents()?;
    ensure!(
        foreign.is_empty(),
        "{} holds data the archiver's store does not write and this tool would not copy: {foreign:?}",
        path.display()
    );
    let (Some(first), Some(last)) = (store.first_height()?, store.latest_height()?) else {
        bail!("{} is empty", path.display());
    };
    let chain_id = store.chain_id()?;
    let shard = Shard {
        path,
        store,
        span: Span { first, last },
    };
    Ok((shard, chain_id))
}

/// Copy every entry of `shard` into `out`, returning how many carried no block hash
/// (legacy 32-byte values; written back in the 64-byte layout with an unknown hash).
fn copy_shard(
    shard: &Shard,
    out: &RootStore,
    batch_size: NonZeroU64,
    copied: &mut u64,
    started: Instant,
) -> Result<u64> {
    let mut legacy = 0;
    for Span { first, last } in chunks(shard.span, batch_size) {
        let entries = shard.store.get_range(first, last)?;
        ensure!(
            entries.len() as u64 == last - first + 1,
            "{}: read {} entries for {first}..={last}; the database changed since it was checked",
            shard.path.display(),
            entries.len()
        );
        let batch: Vec<_> = entries
            .into_iter()
            .map(|(height, stored)| {
                if stored.block_hash.is_zero() {
                    legacy += 1;
                }
                (height, stored.root, stored.block_hash)
            })
            .collect();
        out.put_roots(&batch).with_context(|| {
            format!(
                "writing {first}..={last} from {} (overlapping shards must agree)",
                shard.path.display()
            )
        })?;

        let before = *copied;
        *copied += batch.len() as u64;
        if before / PROGRESS_EVERY != *copied / PROGRESS_EVERY {
            let secs = started.elapsed().as_secs_f64().max(f64::EPSILON);
            tracing::info!(
                shard = %shard.path.display(),
                through = last,
                copied = *copied,
                blocks_per_sec = (*copied as f64 / secs) as u64,
                "copying"
            );
        }
    }
    Ok(legacy)
}

/// Re-read `shard` and check that `out` holds the same root at every height, and the same
/// block hash wherever the shard has one (a legacy entry's unknown hash may have been filled
/// in by an overlapping shard). Independent of the write path's own guard.
fn verify_shard(shard: &Shard, out: &RootStore, batch_size: NonZeroU64) -> Result<()> {
    for Span { first, last } in chunks(shard.span, batch_size) {
        let want = shard.store.get_range(first, last)?;
        let got = out.get_range(first, last)?;
        ensure!(
            want.len() as u64 == last - first + 1 && got.len() == want.len(),
            "{}: {first}..={last} has {} entries in the shard and {} in the output",
            shard.path.display(),
            want.len(),
            got.len()
        );
        for ((height, w), (out_height, g)) in want.iter().zip(&got) {
            ensure!(
                height == out_height
                    && w.root == g.root
                    && (w.block_hash.is_zero() || w.block_hash == g.block_hash),
                "{}: output differs at block {height}: shard has {w:?}, output has block {out_height} {g:?}",
                shard.path.display()
            );
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cfg = Config::parse();
    ensure!(
        cfg.dry_run || !cfg.output.exists(),
        "output {} already exists; refusing to merge into an existing database \
         (if it is left over from an interrupted merge, delete it and re-run)",
        cfg.output.display()
    );

    let mut seen = HashSet::new();
    for path in &cfg.inputs {
        let canonical = path
            .canonicalize()
            .with_context(|| format!("input {}", path.display()))?;
        ensure!(
            seen.insert(canonical),
            "input {} is given more than once",
            path.display()
        );
    }

    let mut shards = Vec::with_capacity(cfg.inputs.len());
    let mut pins = Vec::with_capacity(cfg.inputs.len());
    for path in &cfg.inputs {
        let (shard, chain_id) = open_shard(path.clone())?;
        pins.push((shard.path.clone(), chain_id));
        shards.push(shard);
    }
    let chain_id = resolve_chain_id(&pins, cfg.chain_id)?;

    shards.sort_by_key(|s| s.span.first);
    for shard in &shards {
        let gaps = shard
            .store
            .find_gaps(Some(shard.span.first), Some(shard.span.last))?;
        ensure!(
            gaps.is_empty(),
            "{} has gaps inside {}..={}: {gaps:?}",
            shard.path.display(),
            shard.span.first,
            shard.span.last
        );
        tracing::info!(
            shard = %shard.path.display(),
            first = shard.span.first,
            last = shard.span.last,
            blocks = shard.span.len(),
            "shard is gap-free"
        );
    }

    let spans: Vec<Span> = shards.iter().map(|s| s.span).collect();
    let Joins { gaps, overlaps } = joins(&spans);
    ensure!(gaps.is_empty(), "no shard covers these heights: {gaps:?}");
    for overlap in &overlaps {
        tracing::warn!(
            first = overlap.first,
            last = overlap.last,
            "shards overlap; the overlapping heights must match exactly"
        );
    }
    let total = Span {
        first: spans[0].first,
        last: spans
            .iter()
            .map(|s| s.last)
            .max()
            .expect("at least one shard"),
    };
    tracing::info!(
        chain_id,
        first = total.first,
        last = total.last,
        blocks = total.len(),
        shards = shards.len(),
        "merge plan"
    );
    if cfg.dry_run {
        tracing::info!("dry run: not writing {}", cfg.output.display());
        return Ok(());
    }

    let out = RootStore::open(&cfg.output)?;
    out.pin_chain_id(chain_id)?;

    let started = Instant::now();
    let mut copied = 0;
    let mut legacy = 0;
    for shard in &shards {
        legacy += copy_shard(shard, &out, cfg.batch_size, &mut copied, started)?;
    }
    out.flush().await.context("flushing output")?;

    for shard in &shards {
        verify_shard(shard, &out, cfg.batch_size)?;
        tracing::info!(shard = %shard.path.display(), "output matches shard");
    }

    let out_span = (out.first_height()?, out.latest_height()?);
    ensure!(
        out_span == (Some(total.first), Some(total.last)),
        "output spans {out_span:?}, expected {}..={}",
        total.first,
        total.last
    );
    let gaps = out.find_gaps(Some(total.first), Some(total.last))?;
    ensure!(gaps.is_empty(), "output has gaps: {gaps:?}");
    ensure!(
        out.count() as u64 == total.len(),
        "output entry count {} does not match its span of {} blocks",
        out.count(),
        total.len()
    );
    if legacy > 0 {
        tracing::warn!(
            legacy,
            "entries without a stored block hash were copied with an unknown (zero) hash"
        );
    }
    tracing::info!(
        output = %cfg.output.display(),
        first = total.first,
        last = total.last,
        blocks = total.len(),
        chain_id,
        elapsed_secs = started.elapsed().as_secs(),
        "merge complete and verified"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sp_core::H256;

    fn span(first: u64, last: u64) -> Span {
        Span { first, last }
    }

    #[test]
    fn contiguous_spans_join_cleanly() {
        let spans = [span(0, 9), span(10, 19), span(20, 20)];
        assert_eq!(joins(&spans), Joins::default());
    }

    #[test]
    fn gaps_and_overlaps_are_reported() {
        let spans = [span(0, 9), span(12, 20), span(18, 25), span(19, 22)];
        assert_eq!(
            joins(&spans),
            Joins {
                gaps: vec![span(10, 11)],
                overlaps: vec![span(18, 20), span(19, 22)],
            }
        );
    }

    #[test]
    fn a_contained_span_overlaps_and_does_not_hide_a_later_gap() {
        let spans = [span(0, 100), span(10, 20), span(102, 110)];
        assert_eq!(
            joins(&spans),
            Joins {
                gaps: vec![span(101, 101)],
                overlaps: vec![span(10, 20)],
            }
        );
    }

    #[test]
    fn chunks_cover_the_span_exactly() {
        let size = |n| NonZeroU64::new(n).unwrap();
        assert_eq!(
            chunks(span(5, 16), size(5)).collect::<Vec<_>>(),
            vec![span(5, 9), span(10, 14), span(15, 16)]
        );
        assert_eq!(
            chunks(span(7, 7), size(100)).collect::<Vec<_>>(),
            vec![span(7, 7)]
        );
        assert_eq!(
            chunks(span(u64::MAX - 1, u64::MAX), size(u64::MAX)).collect::<Vec<_>>(),
            vec![span(u64::MAX - 1, u64::MAX)]
        );
    }

    #[test]
    fn chain_id_must_agree() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(
            resolve_chain_id(&[(p("a"), Some(56)), (p("b"), Some(56))], None).unwrap(),
            56
        );
        assert!(resolve_chain_id(&[(p("a"), Some(56)), (p("b"), Some(1))], None).is_err());
        assert!(resolve_chain_id(&[(p("a"), Some(56))], Some(1)).is_err());
        assert!(resolve_chain_id(&[(p("a"), None)], None).is_err());
        assert_eq!(
            resolve_chain_id(&[(p("a"), None), (p("b"), Some(56))], Some(56)).unwrap(),
            56
        );
    }

    fn batch(n: u64) -> NonZeroU64 {
        NonZeroU64::new(n).unwrap()
    }

    #[test]
    fn verify_catches_an_output_that_differs() {
        let dir = tempfile::tempdir().unwrap();
        let a = shard_at(dir.path(), "a", 0..=9);
        let out = RootStore::open(dir.path().join("out")).unwrap();
        let mut copied = 0;
        copy_shard(&a, &out, batch(4), &mut copied, Instant::now()).unwrap();
        verify_shard(&a, &out, batch(4)).unwrap();

        // Missing tail.
        out.truncate_above(8).unwrap();
        assert!(verify_shard(&a, &out, batch(4)).is_err());
        // Present but with a different block hash.
        out.put_roots(&[(9, H256::from_low_u64_be(9), H256::repeat_byte(0x77))])
            .unwrap();
        assert!(verify_shard(&a, &out, batch(4)).is_err());
    }

    fn shard_at(
        dir: &std::path::Path,
        name: &str,
        heights: std::ops::RangeInclusive<u64>,
    ) -> Shard {
        let path = dir.join(name);
        let store = RootStore::open(&path).unwrap();
        store.pin_chain_id(56).unwrap();
        let entries: Vec<_> = heights
            .map(|h| {
                (
                    h,
                    H256::from_low_u64_be(h),
                    H256::from_low_u64_be(h + 1_000),
                )
            })
            .collect();
        store.put_roots(&entries).unwrap();
        let span = Span {
            first: entries[0].0,
            last: entries.last().unwrap().0,
        };
        Shard { path, store, span }
    }

    #[test]
    fn copies_every_entry_and_verifies_overlaps() {
        let dir = tempfile::tempdir().unwrap();
        let a = shard_at(dir.path(), "a", 0..=24);
        let b = shard_at(dir.path(), "b", 20..=40);
        let out = RootStore::open(dir.path().join("out")).unwrap();

        let mut copied = 0;
        for shard in [&a, &b] {
            assert_eq!(
                copy_shard(shard, &out, batch(7), &mut copied, Instant::now()).unwrap(),
                0
            );
        }
        for shard in [&a, &b] {
            verify_shard(shard, &out, batch(7)).unwrap();
        }
        assert_eq!(copied, 25 + 21);
        assert_eq!(out.count(), 41);
        assert!(out.find_gaps(Some(0), Some(40)).unwrap().is_empty());
        assert_eq!(
            out.get_range(0, 40).unwrap(),
            a.store
                .get_range(0, 24)
                .unwrap()
                .into_iter()
                .chain(b.store.get_range(25, 40).unwrap())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_disagreeing_overlap_fails_the_copy() {
        let dir = tempfile::tempdir().unwrap();
        let a = shard_at(dir.path(), "a", 0..=10);
        // `b` shares height 10 with `a`, but with a different root.
        let mut b = shard_at(dir.path(), "b", 11..=20);
        b.store
            .put_roots(&[(10, H256::repeat_byte(0xee), H256::from_low_u64_be(1_010))])
            .unwrap();
        b.span.first = 10;
        let out = RootStore::open(dir.path().join("out")).unwrap();

        let mut copied = 0;
        copy_shard(&a, &out, batch(100), &mut copied, Instant::now()).unwrap();
        let err = copy_shard(&b, &out, batch(100), &mut copied, Instant::now()).unwrap_err();
        assert!(
            err.chain()
                .any(|e| e.downcast_ref::<store::StoreError>().is_some()),
            "{err:?}"
        );
    }
}
