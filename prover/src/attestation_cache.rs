use anyhow::Result;
use attestor_primitives::{AttestationCheckpoint, ChainId, Digest, SignedAttestation};
use hex::ToHex;
use sp_core::H256;
use std::marker::PhantomData;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::{
    cc3,
    postgres::{
        self, attestation,
        attestationcheckpoint::{self, AttestationCheckpoint as DbCheckpoint},
        blockwithdigest,
        cachedupto::{currently_cached_up_to, mark_cached_up_to, CachedUpTo},
        db::PgPool,
    },
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

    pub async fn checkpoint_digest_exists(&self, digest: Digest) -> Result<bool> {
        let mut connection = self.pool.get().await?;

        attestationcheckpoint::exists_by_digest(&mut connection, digest.encode_hex()).await
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

    pub async fn insert_checkpoint(
        &self,
        checkpoint: AttestationCheckpoint,
        chain_id: ChainId,
    ) -> Result<()> {
        let mut connection = self.pool.get().await?;
        let db_checkpoint = DbCheckpoint::from_on_chain(&checkpoint, chain_id);
        let checkpoint_block_number = db_checkpoint.block_number;
        attestationcheckpoint::insert(&mut connection, db_checkpoint).await?;

        // Checkpoints should be strictly from earlier block numbers than attestations. So
        // we remove all attestations older than this new checkpoint from storage.
        attestation::remove_all_before(
            &mut connection,
            checkpoint_block_number,
            postgres::convert(chain_id),
        )
        .await?;

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

    pub async fn currently_cached_up_to(&self) -> Result<Option<CachedUpTo>> {
        let mut connection = self.pool.get().await?;
        Ok(currently_cached_up_to(&mut connection).await)
    }

    pub async fn mark_cached_up_to(&self, cached_up_to: H256) -> Result<()> {
        let mut connection = self.pool.get().await?;
        mark_cached_up_to(&mut connection, cached_up_to).await
    }
}

pub async fn sync_cache(
    chain_id: ChainId,
    attestations_cache: &AttestationCacheType,
    cc3_client: &cc3::Client,
) -> Result<()> {
    // Start subscription for new attestations and checkpoints
    let (attestation_tx, mut attestation_rx) = mpsc::unbounded_channel();
    debug!("Created unbounded attestation cache buffer",);

    let (checkpoint_tx, mut checkpoint_rx) = mpsc::unbounded_channel();
    debug!("Created unbounded attestation checkpoint cache buffer",);

    // Run sub in background and allow server to continue doing other work
    let client = cc3_client.clone();
    let sync_handle = tokio::spawn(async move {
        client
            .start_attestation_sub(attestation_tx, checkpoint_tx, chain_id)
            .await
    });

    // Wait on the channels for new attestations and checkpoints
    loop {
        tokio::select! {
            maybe_attestation = attestation_rx.recv() => {
                let Some(attestation) = maybe_attestation else { break; };

                // check if exists in cache
                if attestations_cache
                    .attestation_digest_exists(attestation.digest())
                    .await?
                {
                    warn!("Attestation already exists in cache, skipping");
                    continue;
                }

                attestations_cache.insert_attestation(attestation).await?;
            },
            maybe_checkpoint = checkpoint_rx.recv() => {
                let Some((checkpoint, chain_id)) = maybe_checkpoint else { break; };

                // check if exists in cache
                if attestations_cache
                    .checkpoint_digest_exists(checkpoint.digest)
                    .await?
                {
                    warn!("Checkpoint already exists in cache, skipping");
                    continue;
                }

                attestations_cache.insert_checkpoint(checkpoint.clone(), chain_id).await?;

                attestations_cache
                    .mark_cached_up_to(checkpoint.digest)
                    .await?;
            }
        }
    }

    sync_handle.await??;

    Ok(())
}

/// This process has two main procedures that are quite similar. First, get the last
/// attestation committed on chain and step backwards along the attestation chain
/// using `prev_digest` fields. When we run out of attestations to sync, the
/// final attestation's `prev_digest` field should point to the first checkpoint.
///
/// Secondly, we step backwards through checkpoints, again using `prev_digest` fields.
/// We stop when we reach the very first checkpoint.
///
/// Upon the successful conclusion of cache building the digest of the most recent
/// checkpoint will be recorded in the `CachedUpTo` table. Future
/// cache building passes then stop early when encountering a checkpoint matching
/// that digest.
pub async fn build_historical_cache_for_chain(
    chain: ChainId,
    attestations_cache: AttestationCacheType,
    cc3_client: CcClientArc,
) -> Result<()> {
    info!("Building historical cache for chain: {}", chain);
    let last_digest = cc3_client.fetch_last_digest(chain).await?;

    let last_digest = if let Some(digest) = last_digest {
        info!("Starting to sync from: {}", digest);
        digest
    } else {
        error!("No historical attestations found for chain: {}", chain);
        return Ok(());
    };

    let mut maybe_digest = Some(last_digest);

    cache_historical_attestations(
        &mut maybe_digest,
        last_digest,
        cc3_client.clone(),
        attestations_cache.clone(),
        chain,
    )
    .await?;

    cache_historical_checkpoints(
        &mut maybe_digest,
        last_digest,
        cc3_client,
        attestations_cache,
        chain,
    )
    .await?;

    info!("Finished building historical cache for chain: {}", chain);

    Ok(())
}

async fn cache_historical_attestations(
    maybe_digest: &mut Option<H256>,
    last_digest: H256,
    cc3_client: CcClientArc,
    attestations_cache: AttestationCacheType,
    chain: ChainId,
) -> Result<bool> {
    loop {
        // Fetch next attestation to put in cache
        let attestation = if let Some(digest) = maybe_digest {
            info!("Fetching attestation with digest: {}", digest);
            let att_fetch_outcome = cc3_client.get_attestation_by_digest(chain, *digest).await?;
            if let Some(attestation) = att_fetch_outcome {
                attestation
            } else {
                info!("Couldn't fetch attestation with given digest: {:?} \nChecking if digest matches
                a checkpoint instead.", digest);
                return Ok(true);
            }
        } else {
            info!("Reached the front of the chain, stopping fetching more historical attestations");
            attestations_cache.mark_cached_up_to(last_digest).await?;
            return Ok(false);
        };

        // Traverse backwards along attestation chain. We update this here
        // rather than at the end of the loop to avoid an unnecessary clone()
        *maybe_digest = attestation.attestation.prev_digest;

        // Check if the attestation already exists in the cache
        let exists_in_cache = attestations_cache
            .attestation_digest_exists(attestation.attestation.digest())
            .await?;
        info!(
            "Checking if attestation {} exists in cache: {}",
            attestation.attestation.digest(),
            exists_in_cache
        );

        if exists_in_cache {
            info!(
                "Digest {} already exists in cache, skipping insertion",
                attestation.attestation.digest()
            );
        } else {
            // Insert the attestation into the cache
            info!(
                "Inserting attestation with digest({}) for chain: {}, blocknumber: {} into cache",
                attestation.attestation.digest(),
                attestation.chain_id(),
                attestation.header_number(),
            );
            attestations_cache.insert_attestation(attestation).await?;
        }
    }
}

async fn cache_historical_checkpoints(
    maybe_digest: &mut Option<H256>,
    last_digest: H256,
    cc3_client: CcClientArc,
    attestations_cache: AttestationCacheType,
    chain: ChainId,
) -> Result<()> {
    // All checkpoints prior to this one don't need to be cached. We already have them!
    let cached_up_to = attestations_cache.currently_cached_up_to().await?;

    // If digest is full after finishing syncing attestations, then that
    // means `prev_digest` of the final attestation pointed to the first
    // checkpoint. Thus we have at least one checkpoint to be cached. We
    // continue by caching all checkpoints starting with the first.
    while let Some(digest) = *maybe_digest {
        if Some(digest.into()) == cached_up_to {
            info!(
                "Current digest matches the last digest up to which we have already cached all checkpoints {}.\n
                Stopping fetching more historical checkpoints",
                digest
            );
            attestations_cache.mark_cached_up_to(last_digest).await?;
            return Ok(());
        }

        // Check whether digest matches a checkpoint on-chain
        let checkpoint = cc3_client
            .get_checkpoint_by_digest(chain, digest)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Checkpoint corresponding to digest not found. Digest: {:?}",
                    digest
                )
            })?;

        // Check if checkpoint is already cached
        let exists_in_cache = attestations_cache.checkpoint_digest_exists(digest).await?;
        info!(
            "Checking if checkpoint {} exists in cache: {}",
            digest, exists_in_cache
        );

        // Traverse backwards along the checkpoint chain.
        *maybe_digest = checkpoint.prev_digest;

        // Insert checkpoint into cache if not present
        if exists_in_cache {
            info!(
                "Checkpoint {} already exists in cache, skipping insertion",
                digest
            );
        } else {
            info!(
                "Inserting checkpoint with digest({}) for chain: {}, blocknumber: {} into cache",
                digest, chain, checkpoint.block_number,
            );
            attestations_cache
                .insert_checkpoint(checkpoint, chain)
                .await?;
        }

        if maybe_digest.is_none() {
            info!("Reached the front of the chain, stopping fetching more historical attestations");
            attestations_cache.mark_cached_up_to(last_digest).await?;
        }
    }

    Ok(())
}
