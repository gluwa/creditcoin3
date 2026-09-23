//! Chains multiple Sled databases into a single, height-ordered `RootSource`.
//!
//! Block-root data can end up split across several separately-ingested Sled databases
//! covering different (and possibly overlapping) height windows. `ChainedSledSource` opens
//! each one, orders them by starting height, resolves any overlaps in favor of the
//! earlier-starting database, and presents the result as one logical `RootSource` so the rest
//! of the pipeline never needs to know it isn't reading from a single database.

use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use tracing::warn;

use super::sled::SledSource;
use super::{RootInfo, RootSource};

/// A Sled source restricted to the height range it contributed after overlap resolution.
struct UsableSource {
    source: SledSource,
    usable_start: u64,
    usable_end: u64,
}

/// One input database's own `[first, last]` span, prior to overlap resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DbSpan {
    index: usize,
    first: u64,
    last: u64,
}

/// A detected overlap between two input databases (identified by their original index).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Overlap {
    winner_index: usize,
    loser_index: usize,
    overlap_start: u64,
    overlap_end: u64,
}

/// How one input database resolved after ordering + overlap resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpanResolution {
    /// Kept, restricted to `[usable_start, usable_end]`.
    Kept {
        index: usize,
        usable_start: u64,
        usable_end: u64,
    },
    /// Fully contained within earlier (winning) database(s); contributes nothing.
    Shadowed { index: usize },
}

/// Order `spans` by starting height and compute each one's usable (post-overlap) range.
///
/// Pure geometry, no I/O, so it's directly unit-testable. The earlier-starting database always
/// keeps its full advertised range; a later-starting database loses the overlapping prefix (or
/// all of itself, if fully contained in an earlier one).
fn resolve_spans(mut spans: Vec<DbSpan>) -> (Vec<SpanResolution>, Vec<Overlap>) {
    spans.sort_by_key(|s| s.first);

    let mut resolutions = Vec::with_capacity(spans.len());
    let mut overlaps = Vec::new();
    // (index of the database that currently owns the highest covered height, that height)
    let mut running_max: Option<(usize, u64)> = None;

    for span in spans {
        if let Some((winner_index, max_covered)) = running_max {
            if span.first <= max_covered {
                overlaps.push(Overlap {
                    winner_index,
                    loser_index: span.index,
                    overlap_start: span.first,
                    overlap_end: max_covered.min(span.last),
                });
            }
        }

        let usable_start = running_max.map_or(span.first, |(_, m)| span.first.max(m + 1));
        let usable_end = span.last;

        resolutions.push(if usable_start <= usable_end {
            SpanResolution::Kept {
                index: span.index,
                usable_start,
                usable_end,
            }
        } else {
            SpanResolution::Shadowed { index: span.index }
        });

        running_max = Some(match running_max {
            Some((idx, m)) if m >= span.last => (idx, m),
            _ => (span.index, span.last),
        });
    }

    (resolutions, overlaps)
}

/// A `RootSource` backed by multiple Sled databases, chained together by height range.
pub struct ChainedSledSource {
    /// Ascending by `usable_start`, non-overlapping, non-empty.
    sources: Vec<UsableSource>,
}

impl ChainedSledSource {
    /// Open the Sled databases at `paths`, order and validate them, and chain them into a
    /// single `RootSource`.
    ///
    /// Each database must be non-empty and internally gapless. Overlapping ranges between
    /// databases are resolved in favor of the earlier-starting one and logged as warnings; if
    /// `fail_on_overlap` is set, any overlap instead causes this to return an error.
    pub fn open(paths: &[PathBuf], fail_on_overlap: bool) -> Result<Self> {
        if paths.is_empty() {
            bail!("At least one Sled database path is required");
        }

        let mut opened: Vec<Option<SledSource>> = Vec::with_capacity(paths.len());
        let mut spans = Vec::with_capacity(paths.len());

        for (index, path) in paths.iter().enumerate() {
            let source = SledSource::open(path)
                .with_context(|| format!("Failed to open Sled database at {}", path.display()))?;

            let first = source
                .first()?
                .ok_or_else(|| anyhow!("Sled database at {} is empty", path.display()))?;
            let last = source
                .last()?
                .ok_or_else(|| anyhow!("Sled database at {} is empty", path.display()))?;

            source
                .assert_gapless(first.height, last.height)
                .with_context(|| {
                    format!(
                        "Sled database at {} failed completeness check",
                        path.display()
                    )
                })?;

            spans.push(DbSpan {
                index,
                first: first.height,
                last: last.height,
            });
            opened.push(Some(source));
        }

        let (resolutions, overlaps) = resolve_spans(spans);

        for overlap in &overlaps {
            warn!(
                "Sled databases overlap: {} (winner) covers heights up to {}, so {} loses \
                 heights [{}, {}]",
                paths[overlap.winner_index].display(),
                overlap.overlap_end,
                paths[overlap.loser_index].display(),
                overlap.overlap_start,
                overlap.overlap_end,
            );
        }

        if fail_on_overlap && !overlaps.is_empty() {
            bail!(
                "Detected {} overlapping range(s) between the provided Sled databases and \
                 --fail-on-overlap is set; refusing to proceed",
                overlaps.len()
            );
        }

        let mut sources = Vec::with_capacity(resolutions.len());
        for resolution in resolutions {
            match resolution {
                SpanResolution::Shadowed { index } => {
                    warn!(
                        "Sled database at {} is fully shadowed by earlier database(s); \
                         excluding it from the chain",
                        paths[index].display()
                    );
                }
                SpanResolution::Kept {
                    index,
                    usable_start,
                    usable_end,
                } => {
                    let source = opened[index]
                        .take()
                        .expect("each database index is resolved at most once");
                    sources.push(UsableSource {
                        source,
                        usable_start,
                        usable_end,
                    });
                }
            }
        }

        // `resolve_spans` sorts by starting height, and kept entries are strictly increasing in
        // both start and end (each one starts after the previous one's end), so `sources` is
        // already in ascending, non-overlapping order.
        Ok(Self { sources })
    }

    fn find(&self, height: u64) -> Option<&UsableSource> {
        self.sources
            .iter()
            .find(|entry| entry.usable_start <= height && height <= entry.usable_end)
    }
}

impl RootSource for ChainedSledSource {
    fn get(&self, height: u64) -> Result<Option<RootInfo>> {
        match self.find(height) {
            Some(entry) => entry.source.get(height),
            None => Ok(None),
        }
    }

    fn get_range(&self, start_height: u64, end_height: u64) -> Result<Vec<RootInfo>> {
        self.iter_range(start_height, end_height).collect()
    }

    fn first(&self) -> Result<Option<RootInfo>> {
        match self.sources.first() {
            Some(entry) => entry.source.first(),
            None => Ok(None),
        }
    }

    fn last(&self) -> Result<Option<RootInfo>> {
        match self.sources.last() {
            Some(entry) => entry.source.last(),
            None => Ok(None),
        }
    }

    fn iter_range(
        &self,
        start_height: u64,
        end_height: u64,
    ) -> Box<dyn Iterator<Item = Result<RootInfo>> + '_> {
        let mut iters: Vec<Box<dyn Iterator<Item = Result<RootInfo>> + '_>> = Vec::new();
        let mut cursor = start_height;

        for entry in &self.sources {
            if cursor > end_height {
                break;
            }
            if entry.usable_end < cursor || entry.usable_start > end_height {
                continue;
            }

            let clip_start = entry.usable_start.max(cursor);
            let clip_end = entry.usable_end.min(end_height);

            if clip_start > cursor {
                let gap_start = cursor;
                let gap_end = clip_start - 1;
                iters.push(Box::new(std::iter::once(Err(anyhow!(
                    "No Sled database covers heights [{gap_start}, {gap_end}] \
                     (requested range [{start_height}, {end_height}])"
                )))));
            }

            iters.push(entry.source.iter_range(clip_start, clip_end));
            cursor = clip_end + 1;
        }

        if cursor <= end_height {
            iters.push(Box::new(std::iter::once(Err(anyhow!(
                "No Sled database covers heights [{cursor}, {end_height}] \
                 (requested range [{start_height}, {end_height}])"
            )))));
        }

        Box::new(iters.into_iter().flatten())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // --- Pure geometry tests (no I/O) ---

    fn span(index: usize, first: u64, last: u64) -> DbSpan {
        DbSpan { index, first, last }
    }

    #[test]
    fn test_resolve_no_overlap_contiguous() {
        let (resolutions, overlaps) = resolve_spans(vec![span(0, 0, 999), span(1, 1000, 1999)]);
        assert!(overlaps.is_empty());
        assert_eq!(
            resolutions,
            vec![
                SpanResolution::Kept {
                    index: 0,
                    usable_start: 0,
                    usable_end: 999
                },
                SpanResolution::Kept {
                    index: 1,
                    usable_start: 1000,
                    usable_end: 1999
                },
            ]
        );
    }

    #[test]
    fn test_resolve_no_overlap_with_gap_is_not_reported_as_overlap() {
        let (resolutions, overlaps) = resolve_spans(vec![span(0, 0, 500), span(1, 600, 999)]);
        assert!(overlaps.is_empty());
        assert_eq!(
            resolutions,
            vec![
                SpanResolution::Kept {
                    index: 0,
                    usable_start: 0,
                    usable_end: 500
                },
                SpanResolution::Kept {
                    index: 1,
                    usable_start: 600,
                    usable_end: 999
                },
            ]
        );
    }

    #[test]
    fn test_resolve_adjacent_overlap_earlier_db_wins() {
        let (resolutions, overlaps) = resolve_spans(vec![span(0, 0, 1000), span(1, 500, 1500)]);
        assert_eq!(
            overlaps,
            vec![Overlap {
                winner_index: 0,
                loser_index: 1,
                overlap_start: 500,
                overlap_end: 1000
            }]
        );
        assert_eq!(
            resolutions,
            vec![
                SpanResolution::Kept {
                    index: 0,
                    usable_start: 0,
                    usable_end: 1000
                },
                SpanResolution::Kept {
                    index: 1,
                    usable_start: 1001,
                    usable_end: 1500
                },
            ]
        );
    }

    #[test]
    fn test_resolve_fully_shadowed_db_is_dropped() {
        let (resolutions, overlaps) = resolve_spans(vec![span(0, 0, 2000), span(1, 500, 800)]);
        assert_eq!(
            overlaps,
            vec![Overlap {
                winner_index: 0,
                loser_index: 1,
                overlap_start: 500,
                overlap_end: 800
            }]
        );
        assert_eq!(
            resolutions,
            vec![
                SpanResolution::Kept {
                    index: 0,
                    usable_start: 0,
                    usable_end: 2000
                },
                SpanResolution::Shadowed { index: 1 },
            ]
        );
    }

    #[test]
    fn test_resolve_three_way_shadow_then_resume() {
        // db1 is fully contained within db0; db2 starts inside db0's range too, so it loses
        // its overlapping prefix but resumes contributing once past db0's end.
        let (resolutions, overlaps) = resolve_spans(vec![
            span(0, 0, 2000),
            span(1, 500, 800),
            span(2, 1000, 3000),
        ]);
        assert_eq!(
            overlaps,
            vec![
                Overlap {
                    winner_index: 0,
                    loser_index: 1,
                    overlap_start: 500,
                    overlap_end: 800
                },
                Overlap {
                    winner_index: 0,
                    loser_index: 2,
                    overlap_start: 1000,
                    overlap_end: 2000
                },
            ]
        );
        assert_eq!(
            resolutions,
            vec![
                SpanResolution::Kept {
                    index: 0,
                    usable_start: 0,
                    usable_end: 2000
                },
                SpanResolution::Shadowed { index: 1 },
                SpanResolution::Kept {
                    index: 2,
                    usable_start: 2001,
                    usable_end: 3000
                },
            ]
        );
    }

    #[test]
    fn test_resolve_unsorted_input_is_sorted_by_start() {
        let (resolutions, overlaps) = resolve_spans(vec![span(0, 1000, 1999), span(1, 0, 999)]);
        assert!(overlaps.is_empty());
        assert_eq!(
            resolutions,
            vec![
                SpanResolution::Kept {
                    index: 1,
                    usable_start: 0,
                    usable_end: 999
                },
                SpanResolution::Kept {
                    index: 0,
                    usable_start: 1000,
                    usable_end: 1999
                },
            ]
        );
    }

    // --- RootSource-level tests against real tempdir Sled databases ---

    /// Write a Sled database at `path` with one entry per height in `heights`. The digest's
    /// last byte is set to `marker` so tests can tell which database answered a query.
    fn write_sled_db(path: &std::path::Path, heights: std::ops::RangeInclusive<u64>, marker: u8) {
        let db = sled::open(path).unwrap();
        for height in heights {
            let key = height.to_be_bytes();
            let mut digest = [0u8; 32];
            digest[0..8].copy_from_slice(&key);
            digest[31] = marker;
            db.insert(key, &digest[..]).unwrap();
        }
        db.flush().unwrap();
    }

    /// Open a chain, tolerating a Sled file lock the previous handle has not released yet.
    /// Mirrors `sled.rs`'s `open_after_close` helper for the same reason (see there).
    fn open_chain_after_close(paths: &[PathBuf], fail_on_overlap: bool) -> ChainedSledSource {
        const ATTEMPTS: usize = 20;
        const WAIT: std::time::Duration = std::time::Duration::from_millis(50);

        let mut last = None;
        for _ in 0..ATTEMPTS {
            match ChainedSledSource::open(paths, fail_on_overlap) {
                Ok(source) => return source,
                Err(e) => {
                    last = Some(e);
                    std::thread::sleep(WAIT);
                }
            }
        }
        panic!(
            "could not open chain after {ATTEMPTS} attempts over {:?}: {:?}",
            WAIT * ATTEMPTS as u32,
            last.expect("at least one attempt failed"),
        )
    }

    #[test]
    fn test_chained_two_dbs_no_overlap() {
        let dir = tempdir().unwrap();
        let path_a = dir.path().join("a");
        let path_b = dir.path().join("b");
        write_sled_db(&path_a, 0..=499, 0xAA);
        write_sled_db(&path_b, 500..=999, 0xBB);

        let chain = open_chain_after_close(&[path_a, path_b], false);

        assert_eq!(chain.first().unwrap().unwrap().height, 0);
        assert_eq!(chain.last().unwrap().unwrap().height, 999);
        assert_eq!(chain.get(250).unwrap().unwrap().digest.as_bytes()[31], 0xAA);
        assert_eq!(chain.get(750).unwrap().unwrap().digest.as_bytes()[31], 0xBB);

        // A range spanning the chain boundary stitches both databases together in order.
        let roots = chain.get_range(490, 510).unwrap();
        assert_eq!(roots.len(), 21);
        for (i, root) in roots.iter().enumerate() {
            assert_eq!(root.height, 490 + i as u64);
        }
        assert_eq!(roots[9].digest.as_bytes()[31], 0xAA); // height 499
        assert_eq!(roots[10].digest.as_bytes()[31], 0xBB); // height 500
    }

    #[test]
    fn test_chained_overlap_earlier_db_wins_reads() {
        let dir = tempdir().unwrap();
        let path_a = dir.path().join("a");
        let path_b = dir.path().join("b");
        write_sled_db(&path_a, 0..=1000, 0xAA);
        write_sled_db(&path_b, 500..=1500, 0xBB);

        let chain = open_chain_after_close(&[path_a, path_b], false);

        assert_eq!(chain.first().unwrap().unwrap().height, 0);
        assert_eq!(chain.last().unwrap().unwrap().height, 1500);
        // Overlapping height: the earlier-starting database (a) wins.
        assert_eq!(chain.get(700).unwrap().unwrap().digest.as_bytes()[31], 0xAA);
        // Only b covers this height.
        assert_eq!(
            chain.get(1200).unwrap().unwrap().digest.as_bytes()[31],
            0xBB
        );
    }

    #[test]
    fn test_chained_fail_on_overlap_errors() {
        let dir = tempdir().unwrap();
        let path_a = dir.path().join("a");
        let path_b = dir.path().join("b");
        write_sled_db(&path_a, 0..=1000, 0xAA);
        write_sled_db(&path_b, 500..=1500, 0xBB);

        const ATTEMPTS: usize = 20;
        const WAIT: std::time::Duration = std::time::Duration::from_millis(50);
        let mut result = ChainedSledSource::open(&[path_a.clone(), path_b.clone()], true);
        for _ in 0..ATTEMPTS {
            if result.is_err() {
                break;
            }
            std::thread::sleep(WAIT);
            result = ChainedSledSource::open(&[path_a.clone(), path_b.clone()], true);
        }
        assert!(result.is_err(), "expected an error due to overlap");
    }

    #[test]
    fn test_chained_gap_between_dbs_surfaces_on_range_read() {
        let dir = tempdir().unwrap();
        let path_a = dir.path().join("a");
        let path_b = dir.path().join("b");
        write_sled_db(&path_a, 0..=499, 0xAA);
        write_sled_db(&path_b, 600..=999, 0xBB); // gap: 500..=599 missing

        let chain = open_chain_after_close(&[path_a, path_b], false);

        assert_eq!(chain.first().unwrap().unwrap().height, 0);
        assert_eq!(chain.last().unwrap().unwrap().height, 999);
        // A single lookup inside the gap is just "not found".
        assert!(chain.get(550).unwrap().is_none());
        // A range read spanning the gap is an error, matching single-DB gap semantics.
        assert!(chain.get_range(0, 999).is_err());
    }

    #[test]
    fn test_chained_three_dbs_shadowed_db_excluded_from_reads() {
        let dir = tempdir().unwrap();
        let path_a = dir.path().join("a");
        let path_b = dir.path().join("b");
        let path_c = dir.path().join("c");
        write_sled_db(&path_a, 0..=2000, 0xAA);
        write_sled_db(&path_b, 500..=800, 0xBB); // fully shadowed by a
        write_sled_db(&path_c, 1000..=3000, 0xCC);

        let chain = open_chain_after_close(&[path_a, path_b, path_c], false);

        assert_eq!(chain.first().unwrap().unwrap().height, 0);
        assert_eq!(chain.last().unwrap().unwrap().height, 3000);
        assert_eq!(chain.get(600).unwrap().unwrap().digest.as_bytes()[31], 0xAA);
        assert_eq!(
            chain.get(2500).unwrap().unwrap().digest.as_bytes()[31],
            0xCC
        );

        let roots = chain.get_range(0, 3000).unwrap();
        assert_eq!(roots.len(), 3001);
        for (i, root) in roots.iter().enumerate() {
            assert_eq!(root.height, i as u64);
        }
    }
}
