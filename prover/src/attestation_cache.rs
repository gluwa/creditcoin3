use anyhow::Result;
use attestor_primitives::{ChainId, Digest, SignedAttestation};
use hex::ToHex;
use sp_core::H256;
use std::marker::PhantomData;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::{
    cc3,
    postgres::{attestation, blockwithdigest, db::PgPool},
    AttestationCacheType, CcClientArc,
};

#[derive(Clone)]
pub struct AttestationCache<H, A> {
    pool: PgPool,
    phantom: PhantomData<(H, A)>,
}

impl<H, A> AttestationCache<H, A> {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        AttestationCache {
            pool,
            phantom: PhantomData,
        }
    }
}

impl<H, A> AttestationCache<H, A>
where
    H: AsRef<[u8]> + Clone + Copy,
    A: AsRef<[u8]> + Clone,
{
    pub async fn get_attestation_by_digest(
        &self,
        digest: Digest,
    ) -> Result<attestation::Attestation> {
        let mut connection = self.pool.get().await?;
        let attestation = attestation::get_by_digest(&mut connection, digest.encode_hex()).await?;

        Ok(attestation)
    }

    pub async fn attestation_digest_exists(&self, digest: Digest) -> Result<bool> {
        let mut connection = self.pool.get().await?;

        attestation::exists_by_digest(&mut connection, digest.encode_hex()).await
    }

    pub async fn get_attestation_by_header_number(
        &self,
        header_number: i64,
        chain_id: i64,
    ) -> Result<attestation::Attestation> {
        let mut connection = self.pool.get().await?;
        let attestation =
            attestation::get_by_header_number(&mut connection, header_number, chain_id).await?;

        Ok(attestation)
    }

    pub async fn insert_attestation(&self, attestation: SignedAttestation<H, A>) -> Result<()> {
        let mut connection = self.pool.get().await?;
        attestation::insert(&mut connection, attestation.into()).await?;

        Ok(())
    }

    pub async fn first_attestation_exists(&self, chain_id: ChainId) -> Result<bool> {
        let mut connection = self.pool.get().await?;
        attestation::first_digest_exists(&mut connection, chain_id).await
    }

    pub async fn last_synced_attestation(
        &self,
        chain_id: ChainId,
    ) -> Result<Option<attestation::Attestation>> {
        let mut connection = self.pool.get().await?;
        attestation::last_synced(&mut connection, chain_id).await
    }

    pub async fn get_attestation_fragment(
        &self,
        chain_id: ChainId,
        start: u64,
        end: u64,
    ) -> Result<Vec<blockwithdigest::BlockWithDigest>> {
        let mut connection = self.pool.get().await?;
        blockwithdigest::get_blocks_in_range(&mut connection, chain_id, start as i64, end as i64)
            .await
    }

    pub async fn upsert_fragment(
        &self,
        fragment: &Vec<blockwithdigest::BlockWithDigest>,
    ) -> Result<()> {
        let mut connection = self.pool.get().await?;
        blockwithdigest::upsert_fragment_blocks(&mut connection, fragment).await
    }
}

pub async fn sync_cache(
    chain_id: ChainId,
    attestations_cache: &AttestationCacheType,
    cc3_client: &cc3::Client,
) -> Result<()> {
    // Start subscription for new attestations
    let (attestation_tx, mut attestation_rx) = mpsc::channel(100);
    debug!("Created cache buffer with size: {}", 100);

    // Run sub in background and allow server to continue doing other work
    let client = cc3_client.clone();
    let sync_handle =
        tokio::spawn(async move { client.start_attestation_sub(attestation_tx, chain_id).await });

    // Wait on the channel for new attestations
    while let Some(attestation) = attestation_rx.recv().await {
        // check if exists in cache
        if attestations_cache
            .attestation_digest_exists(attestation.digest())
            .await?
        {
            warn!("Attestation already exists in cache, skipping");
            continue;
        }

        attestations_cache.insert_attestation(attestation).await?;
    }

    sync_handle.await??;

    Ok(())
}

pub async fn build_historical_cache_for_chain(
    chain: ChainId,
    attestations_cache: AttestationCacheType,
    cc3_client: CcClientArc,
) -> Result<()> {
    info!("Building historical cache for chain: {}", chain);
    let last_digest = cc3_client.fetch_last_digest(chain).await?;

    if last_digest.is_none() {
        error!("No historical attestations found for chain: {}", chain);
        return Ok(());
    }

    // Get the last attestation
    let mut last_chain_attestation = cc3_client
        .get_attestation_by_digest(chain, last_digest.unwrap())
        .await
        .map_err(|e| anyhow::anyhow!("Error fetching last attestation: {:?}", e))?
        .ok_or_else(|| anyhow::anyhow!("Last attestation not found"))?;

    // Check if the first digest exists (one with prev_digest = Null) (meaning the front of the chain)
    let head_of_chain_exists = attestations_cache.first_attestation_exists(chain).await?;

    // Fetch the last synced attestation from the cache
    let last_attestation_synced_in_cache =
        attestations_cache.last_synced_attestation(chain).await?;

    if !head_of_chain_exists && last_attestation_synced_in_cache.is_some() {
        let digest = H256::from_slice(
            &hex::decode(
                last_attestation_synced_in_cache
                    .unwrap()
                    .prev_digest
                    .unwrap(),
            )
            .map_err(|e| anyhow::anyhow!("Error decoding prev_digest: {:?}", e))?,
        );
        info!("Head of chain not found in cache, but last attestation found in cache, starting to sync from: {}", digest);

        // fetch last attestation from cache
        last_chain_attestation = cc3_client
            .get_attestation_by_digest(chain, digest)
            .await
            .map_err(|e| anyhow::anyhow!("Error fetching last attestation: {:?}", e))?
            .ok_or_else(|| anyhow::anyhow!("Last attestation not found"))?;
    }

    let mut fetch_more = true;
    // Fetch more historical attestations
    while fetch_more {
        let digest = last_chain_attestation.attestation.digest();

        // Check if the digest already exists in the cache
        let exists_in_cache = attestations_cache.attestation_digest_exists(digest).await?;
        info!(
            "Checking if digest {} exists in cache: {}",
            digest, exists_in_cache
        );

        // Check if the digest already exists in the cache and the first digest exists
        // If this digest exists in the cache and the first digest exists, we can stop fetching more
        // because it means we have fetched all the historical attestations
        if exists_in_cache && head_of_chain_exists {
            info!(
                "Digest {} already exists in cache, skipping insertion",
                digest
            );
            fetch_more = false;
        };

        if !exists_in_cache {
            // Insert the attestation into the cache
            info!(
                "Inserting attestation with digest({}) for chain: {}, blocknumber: {} into cache",
                digest,
                last_chain_attestation.chain_id(),
                last_chain_attestation.header_number(),
            );
            attestations_cache
                .insert_attestation(last_chain_attestation.clone())
                .await?;
        }

        // Fetch the next attestation
        if let Some(prev_digest) = last_chain_attestation.attestation.prev_digest {
            info!("Fetching attestation with prev_digest: {}", prev_digest);
            last_chain_attestation = cc3_client
                .get_attestation_by_digest(chain, prev_digest)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Last attestation not found"))?;
        } else {
            info!("Reached the front of the chain, stopping fetching more historical attestations");
            fetch_more = false;
        }
    }

    info!("Finished building historical cache for chain: {}", chain);

    Ok(())
}
