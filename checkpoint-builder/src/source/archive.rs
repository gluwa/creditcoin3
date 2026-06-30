//! Archiver HTTP API source for reading block root data.

use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde::Deserialize;

use super::{RootInfo, RootSource};

/// Maximum blocks per single API request (archiver enforces < 1_000).
const MAX_BATCH: u64 = 1_000;

#[derive(Deserialize)]
struct RootEntry {
    block_number: u64,
    merkle_root: String,
}

#[derive(Deserialize)]
struct LatestResponse {
    latest_block: Option<u64>,
}

#[derive(Deserialize)]
struct StatusResponse {
    latest_archived_block: Option<u64>,
    total_blocks: usize,
}

/// Source for reading block roots from an archiver HTTP API.
pub struct ArchiveSource {
    client: Client,
    base_url: url::Url,
}

/// Validates that an archiver API response covers exactly the requested range with no gaps,
/// duplicates, or out-of-range entries.
///
/// The archiver is an external, potentially untrusted source. This check ensures that a
/// faulty or adversarial API cannot feed the checkpoint builder malformed root sequences that
/// would silently corrupt the digest chain.
fn validate_range_response(from: u64, to: u64, entries: &[RootInfo]) -> Result<()> {
    let expected_count = (to - from + 1) as usize;
    if entries.len() != expected_count {
        anyhow::bail!(
            "Archiver returned {} entries for range [{from}, {to}], expected {expected_count}",
            entries.len()
        );
    }
    for (i, entry) in entries.iter().enumerate() {
        let expected_height = from + i as u64;
        if entry.height != expected_height {
            anyhow::bail!(
                "Archiver returned unexpected height at index {i}: \
                 expected {expected_height}, got {} (range [{from}, {to}])",
                entry.height
            );
        }
    }
    Ok(())
}

impl ArchiveSource {
    pub fn new(base_url: url::Url) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .context("Failed to build HTTP client")?;
        Ok(Self { client, base_url })
    }

    fn fetch_range(&self, from: u64, to: u64) -> Result<Vec<RootInfo>> {
        let url = format!("{}roots?from={}&to={}", self.base_url, from, to);
        let entries: Vec<RootEntry> = self
            .client
            .get(&url)
            .send()
            .with_context(|| format!("Failed to GET {url}"))?
            .error_for_status()
            .with_context(|| format!("HTTP error from {url}"))?
            .json()
            .with_context(|| format!("Failed to parse JSON from {url}"))?;

        let results = entries
            .into_iter()
            .map(|entry| {
                let hex = entry
                    .merkle_root
                    .strip_prefix("0x")
                    .unwrap_or(&entry.merkle_root);
                let bytes = hex::decode(hex).with_context(|| {
                    format!("Failed to decode merkle_root hex: {}", entry.merkle_root)
                })?;
                if bytes.len() != 32 {
                    anyhow::bail!(
                        "Invalid digest length: expected 32 bytes, got {}",
                        bytes.len()
                    );
                }
                Ok(RootInfo {
                    height: entry.block_number,
                    digest: attestor_primitives::Digest::from_slice(&bytes),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        validate_range_response(from, to, &results).with_context(|| {
            format!("Archive range response validation failed for [{from}, {to}]")
        })?;

        Ok(results)
    }

    fn fetch_all(&self, start: u64, end: u64) -> Result<Vec<RootInfo>> {
        let mut results = Vec::new();
        let mut from = start;
        while from <= end {
            let to = (from + MAX_BATCH - 1).min(end);
            let batch = self.fetch_range(from, to)?;
            results.extend(batch);
            from = to + 1;
        }
        Ok(results)
    }
}

impl RootSource for ArchiveSource {
    fn get(&self, height: u64) -> Result<Option<RootInfo>> {
        Ok(self.fetch_range(height, height)?.into_iter().next())
    }

    fn get_range(&self, start_height: u64, end_height: u64) -> Result<Vec<RootInfo>> {
        self.fetch_all(start_height, end_height)
    }

    fn first(&self) -> Result<Option<RootInfo>> {
        let url = format!("{}status", self.base_url);
        let status: StatusResponse = self
            .client
            .get(&url)
            .send()
            .with_context(|| format!("Failed to GET {url}"))?
            .error_for_status()
            .with_context(|| format!("HTTP error from {url}"))?
            .json()
            .with_context(|| format!("Failed to parse JSON from {url}"))?;

        match (status.latest_archived_block, status.total_blocks) {
            (Some(latest), total) if total > 0 => {
                let first_height = (latest + 1).saturating_sub(total as u64);
                Ok(self.get(first_height)?)
            }
            _ => Ok(None),
        }
    }

    fn last(&self) -> Result<Option<RootInfo>> {
        let url = format!("{}roots/latest", self.base_url);
        let latest: LatestResponse = self
            .client
            .get(&url)
            .send()
            .with_context(|| format!("Failed to GET {url}"))?
            .error_for_status()
            .with_context(|| format!("HTTP error from {url}"))?
            .json()
            .with_context(|| format!("Failed to parse JSON from {url}"))?;

        match latest.latest_block {
            Some(h) => Ok(self.get(h)?),
            None => Ok(None),
        }
    }

    fn iter_range(
        &self,
        start_height: u64,
        end_height: u64,
    ) -> Box<dyn Iterator<Item = Result<RootInfo>> + '_> {
        match self.fetch_all(start_height, end_height) {
            Ok(items) => Box::new(items.into_iter().map(Ok)),
            Err(e) => Box::new(std::iter::once(Err(e))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use attestor_primitives::Digest;

    fn make_entry(height: u64) -> RootInfo {
        RootInfo {
            height,
            digest: Digest::default(),
        }
    }

    #[test]
    fn test_validate_range_ok() {
        let entries = vec![make_entry(5), make_entry(6), make_entry(7)];
        assert!(validate_range_response(5, 7, &entries).is_ok());
    }

    #[test]
    fn test_validate_range_single_entry() {
        let entries = vec![make_entry(42)];
        assert!(validate_range_response(42, 42, &entries).is_ok());
    }

    #[test]
    fn test_validate_range_too_few_entries() {
        let entries = vec![make_entry(5), make_entry(6)];
        assert!(validate_range_response(5, 7, &entries).is_err());
    }

    #[test]
    fn test_validate_range_too_many_entries() {
        let entries = vec![make_entry(5), make_entry(6), make_entry(7), make_entry(8)];
        assert!(validate_range_response(5, 7, &entries).is_err());
    }

    #[test]
    fn test_validate_range_gap_in_sequence() {
        // block 6 missing, block 8 substituted
        let entries = vec![make_entry(5), make_entry(7), make_entry(8)];
        assert!(validate_range_response(5, 7, &entries).is_err());
    }

    #[test]
    fn test_validate_range_out_of_range_low() {
        // starts one below the requested range
        let entries = vec![make_entry(4), make_entry(5), make_entry(6)];
        assert!(validate_range_response(5, 7, &entries).is_err());
    }

    #[test]
    fn test_validate_range_out_of_range_high() {
        // starts one above the requested range
        let entries = vec![make_entry(6), make_entry(7), make_entry(8)];
        assert!(validate_range_response(5, 7, &entries).is_err());
    }

    #[test]
    fn test_validate_range_empty_response() {
        let entries: Vec<RootInfo> = vec![];
        assert!(validate_range_response(5, 7, &entries).is_err());
    }
}
