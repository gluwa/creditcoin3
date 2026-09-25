//! Canonical-anchor reconciliation.
//!
//! The reorg guard in [`RootStore::put_roots`] only fires when a height that is *already
//! stored* is written again with different content. Two paths never hit it: resuming after a
//! restart and reconnecting after a stream death, both of which continue from `stored tip + 1`
//! and trust that everything at or below the tip is still canonical. If the source chain
//! reorged past the finalization lag while the archiver was away, or the lag was simply set
//! too small, the archive tail sits on an abandoned fork and canonical roots get spliced on
//! top of fork roots without anything noticing.
//!
//! This module closes that hole: before (re)starting the stream, re-fetch the block at the
//! stored tip and compare its hash with the one persisted alongside the root. By default a
//! mismatch is fatal (fail closed). With `--reanchor-max-depth N` the archiver walks back at
//! most `N` stored blocks to find the last canonical one, drops everything above it and
//! resumes from there; the bound keeps a misbehaving RPC from wiping the archive.

use std::future::Future;

use anyhow::{anyhow, Context, Result};
use sp_core::H256;

use crate::store::{RootStore, StoreError};

/// What reconciliation found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// The store holds no roots; nothing to anchor against.
    Empty,
    /// The stored tip matches the canonical block at that height.
    Verified { tip: u64 },
    /// The stored tip predates the block-hash column, so it cannot be checked.
    Unverifiable { tip: u64 },
    /// The tail sat on an abandoned fork; `removed` entries above `tip` were dropped and the
    /// stream must resume from `tip + 1`.
    Reanchored { tip: u64, removed: u64 },
}

impl Anchor {
    /// The height the stream must resume from after reconciliation, if the store is
    /// non-empty.
    pub fn resume_from(&self) -> Option<u64> {
        match self {
            Anchor::Empty => None,
            Anchor::Verified { tip }
            | Anchor::Unverifiable { tip }
            | Anchor::Reanchored { tip, .. } => Some(tip + 1),
        }
    }
}

/// Reconcile the store's tail against the canonical chain.
///
/// `canonical_hash(height)` must return the hash of the canonical block at `height`.
/// `max_depth` is how many stored blocks below the tip may be dropped to find a canonical
/// anchor; `0` means fail closed on any tip mismatch.
pub async fn reconcile<F, Fut>(
    store: &RootStore,
    max_depth: u64,
    mut canonical_hash: F,
) -> Result<Anchor>
where
    F: FnMut(u64) -> Fut,
    Fut: Future<Output = Result<H256>>,
{
    let Some(tip) = store.latest_height()? else {
        return Ok(Anchor::Empty);
    };

    // Candidates, highest first: the tip plus at most `max_depth` stored blocks below it.
    // Gaps below the tip simply yield fewer candidates; they are the backfiller's problem.
    let floor = tip.saturating_sub(max_depth);
    let mut candidates = store.get_range(floor, tip)?;
    candidates.reverse();

    let mut tip_mismatch: Option<(H256, H256)> = None;
    for (height, stored) in candidates {
        if stored.block_hash.is_zero() {
            // Legacy entry without a hash. At the tip that is merely unverifiable; below a
            // mismatching tip it means the fork point cannot be located, so stop here and
            // let the operator decide.
            if height == tip {
                return Ok(Anchor::Unverifiable { tip });
            }
            break;
        }
        let canonical = canonical_hash(height)
            .await
            .with_context(|| format!("fetching canonical block {height} for anchor check"))?;
        if canonical == stored.block_hash {
            if height == tip {
                return Ok(Anchor::Verified { tip });
            }
            let removed = store.truncate_above(height)?;
            return Ok(Anchor::Reanchored {
                tip: height,
                removed,
            });
        }
        if height == tip {
            tip_mismatch = Some((stored.block_hash, canonical));
        }
    }

    let (stored_hash, canonical_hash) =
        tip_mismatch.expect("the tip is always the first candidate and did not match");
    Err(anyhow!(StoreError::AnchorMismatch {
        height: tip,
        stored_hash,
        canonical_hash,
    })
    .context(if max_depth == 0 {
        "refusing to resume on top of a fork; re-run with --reanchor-max-depth N to drop up to N \
         fork blocks and recompute them, or restore the archive from a known-good snapshot"
            .to_string()
    } else {
        format!(
            "no canonical block found within --reanchor-max-depth {max_depth} of the stored tip; \
             the fork is deeper than allowed (or the RPC is serving another chain)"
        )
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn store_with(entries: &[(u64, H256)]) -> (tempfile::TempDir, RootStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = RootStore::open(dir.path().join("t.sled")).unwrap();
        let rows: Vec<_> = entries
            .iter()
            .map(|(h, hash)| (*h, H256::random(), *hash))
            .collect();
        store.put_roots(&rows).unwrap();
        (dir, store)
    }

    fn chain(map: HashMap<u64, H256>) -> impl FnMut(u64) -> std::future::Ready<Result<H256>> {
        move |h| std::future::ready(map.get(&h).copied().ok_or_else(|| anyhow!("no block {h}")))
    }

    #[tokio::test]
    async fn empty_store_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = RootStore::open(dir.path().join("t.sled")).unwrap();
        let a = reconcile(&store, 0, chain(HashMap::new())).await.unwrap();
        assert_eq!(a, Anchor::Empty);
        assert_eq!(a.resume_from(), None);
    }

    #[tokio::test]
    async fn matching_tip_is_verified_without_touching_the_store() {
        let hashes: Vec<H256> = (0..3).map(|_| H256::random()).collect();
        let (_d, store) = store_with(&[(10, hashes[0]), (11, hashes[1]), (12, hashes[2])]);
        let mut calls = 0;
        let a = reconcile(&store, 5, |h| {
            calls += 1;
            std::future::ready(Ok(hashes[(h - 10) as usize]))
        })
        .await
        .unwrap();
        assert_eq!(a, Anchor::Verified { tip: 12 });
        assert_eq!(a.resume_from(), Some(13));
        assert_eq!(calls, 1, "only the tip is fetched when it matches");
        assert_eq!(store.count(), 3);
    }

    #[tokio::test]
    async fn tip_mismatch_fails_closed_by_default() {
        let good = H256::random();
        let (_d, store) = store_with(&[(10, good), (11, H256::random())]);
        let canon: HashMap<u64, H256> = [(10, good), (11, H256::random())].into();
        let err = reconcile(&store, 0, chain(canon)).await.unwrap_err();
        assert!(
            matches!(
                err.downcast_ref::<StoreError>(),
                Some(StoreError::AnchorMismatch { height: 11, .. })
            ),
            "{err:?}"
        );
        assert!(format!("{err:#}").contains("--reanchor-max-depth"));
        assert_eq!(store.count(), 2, "fail closed must not modify the store");
    }

    #[tokio::test]
    async fn reanchors_within_depth_and_truncates_the_fork_tail() {
        let canon: Vec<H256> = (0..5).map(|_| H256::random()).collect(); // heights 10..=14
        let fork: Vec<H256> = (0..2).map(|_| H256::random()).collect(); // stored at 13, 14
        let (_d, store) = store_with(&[
            (10, canon[0]),
            (11, canon[1]),
            (12, canon[2]),
            (13, fork[0]),
            (14, fork[1]),
        ]);
        let map: HashMap<u64, H256> = (10..=14).map(|h| (h, canon[(h - 10) as usize])).collect();
        let a = reconcile(&store, 3, chain(map)).await.unwrap();
        assert_eq!(
            a,
            Anchor::Reanchored {
                tip: 12,
                removed: 2
            }
        );
        assert_eq!(a.resume_from(), Some(13));
        assert_eq!(store.latest_height().unwrap(), Some(12));
        assert_eq!(store.count(), 3);
    }

    #[tokio::test]
    async fn fork_deeper_than_depth_fails_without_truncating() {
        let (_d, store) = store_with(&[
            (10, H256::random()),
            (11, H256::random()),
            (12, H256::random()),
        ]);
        let map: HashMap<u64, H256> = (10..=12).map(|h| (h, H256::random())).collect();
        let err = reconcile(&store, 1, chain(map)).await.unwrap_err();
        assert!(matches!(
            err.downcast_ref::<StoreError>(),
            Some(StoreError::AnchorMismatch { height: 12, .. })
        ));
        assert!(format!("{err:#}").contains("deeper than allowed"));
        assert_eq!(store.count(), 3);
    }

    #[tokio::test]
    async fn legacy_tip_without_hash_is_unverifiable() {
        let (_d, store) = store_with(&[(10, H256::random()), (11, H256::zero())]);
        let a = reconcile(&store, 3, chain(HashMap::new())).await.unwrap();
        assert_eq!(a, Anchor::Unverifiable { tip: 11 });
        assert_eq!(store.count(), 2);
    }

    #[tokio::test]
    async fn legacy_entry_below_a_mismatching_tip_stops_the_walk() {
        let (_d, store) = store_with(&[(10, H256::zero()), (11, H256::random())]);
        let map: HashMap<u64, H256> = [(11, H256::random())].into();
        let err = reconcile(&store, 5, chain(map)).await.unwrap_err();
        assert!(matches!(
            err.downcast_ref::<StoreError>(),
            Some(StoreError::AnchorMismatch { height: 11, .. })
        ));
        assert_eq!(store.count(), 2);
    }

    #[tokio::test]
    async fn rpc_error_propagates_without_truncating() {
        let (_d, store) = store_with(&[(10, H256::random()), (11, H256::random())]);
        let err = reconcile(&store, 5, chain(HashMap::new()))
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("no block 11"));
        assert_eq!(store.count(), 2);
    }
}
