use anyhow::{anyhow, Result};
use attestor_primitives::{
    AttestationCheckpoint, ChainKey, Digest, PalletDigest, SignedAttestation,
};
use diesel_async::AsyncPgConnection;
use hex::ToHex;
use sp_core::H256;
use std::marker::PhantomData;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tracing::{debug, info, warn};

use crate::{
    cc3,
    postgres::{
        self,
        attestation::{self, Error as DbAttestationError},
        attestationcheckpoint::{
            self, AttestationCheckpoint as DbCheckpoint, Error as DbCheckpointError,
        },
        blockwithdigest,
        cachedupto::{currently_cached_up_to, mark_cached_up_to, CachedUpTo},
        db::PgPool,
        from_storage_type, to_storage_type,
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

    pub async fn get_checkpoint_by_digest(&self, digest: String) -> Result<DbCheckpoint> {
        let mut connection = self.pool.get().await?;
        let checkpoint = attestationcheckpoint::get_by_digest(&mut connection, digest).await?;

        Ok(checkpoint)
    }

    pub async fn attestation_digest_exists(&self, digest: PalletDigest) -> Result<bool> {
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
        chain_key: i64,
    ) -> Result<attestation::Attestation> {
        let mut connection = self.pool.get().await?;
        let attestation =
            attestation::get_by_header_number(&mut connection, header_number, chain_key).await?;

        Ok(attestation)
    }

    pub async fn get_checkpoint_by_block_number(
        &self,
        block_number: i64,
        chain_key: i64,
    ) -> Result<DbCheckpoint> {
        let mut connection = self.pool.get().await?;
        let checkpoint =
            attestationcheckpoint::get_by_block_number(&mut connection, block_number, chain_key)
                .await?;

        Ok(checkpoint)
    }

    pub async fn insert_attestation(&self, attestation: SignedAttestation<H, A>) -> Result<()> {
        let mut connection = self.pool.get().await?;
        if let Err(e) = attestation::insert(&mut connection, attestation.into()).await {
            match e {
                DbAttestationError::DuplicateChainKeyAndBlockNumber => {
                    return Err(anyhow!("Inserted attestation with duplicate (chain_key, block_height). Therefore DB contains bad state. Clean DB and run prover to resync. Error: {:?}", e));
                }
                DbAttestationError::Other(e) => {
                    return Err(anyhow!("{:?}", e));
                }
            }
        }

        Ok(())
    }

    pub async fn insert_checkpoint(
        &self,
        checkpoint: AttestationCheckpoint,
        chain_key: ChainKey,
    ) -> Result<()> {
        let mut connection = self.pool.get().await?;
        let db_checkpoint = DbCheckpoint::from_on_chain(&checkpoint, chain_key);
        let checkpoint_block_number = db_checkpoint.block_number;
        if let Err(e) = attestationcheckpoint::insert(&mut connection, db_checkpoint).await {
            match e {
                DbCheckpointError::DuplicateChainKeyAndBlockNumber => {
                    return Err(anyhow!("Inserted attestation with duplicate (chain_key, block_height). Therefore DB contains bad state. Clean DB and run prover to resync. Error: {:?}", e));
                }
                DbCheckpointError::Other(e) => {
                    return Err(anyhow!("{:?}", e));
                }
            }
        }

        // Checkpoints should be strictly from earlier block numbers than attestations. So
        // we remove all attestations older than this new checkpoint from storage.
        attestation::remove_all_before(
            &mut connection,
            checkpoint_block_number,
            postgres::to_storage_type(chain_key),
        )
        .await?;

        Ok(())
    }

    pub async fn first_attestation_exists(&self, chain_key: ChainKey) -> Result<bool> {
        let mut connection = self.pool.get().await?;
        attestation::first_digest_exists(&mut connection, chain_key).await
    }

    pub async fn last_synced_attestation(
        &self,
        chain_key: ChainKey,
    ) -> Result<Option<attestation::Attestation>> {
        let mut connection = self.pool.get().await?;
        attestation::last_synced(&mut connection, chain_key).await
    }

    pub async fn earliest_attestation(
        &self,
        chain_key: ChainKey,
    ) -> Result<Option<attestation::Attestation>> {
        let mut connection = self.pool.get().await?;
        attestation::earliest_attestation(&mut connection, chain_key).await
    }

    pub async fn get_attestation_fragment(
        &self,
        chain_key: ChainKey,
        start: u64,
        end: u64,
    ) -> Result<Vec<blockwithdigest::BlockWithDigest>> {
        let mut connection = self.pool.get().await?;
        blockwithdigest::get_blocks_in_range(&mut connection, chain_key, start as i64, end as i64)
            .await
    }

    pub async fn upsert_fragment(
        &self,
        fragment: &Vec<blockwithdigest::BlockWithDigest>,
    ) -> Result<()> {
        let mut connection = self.pool.get().await?;
        blockwithdigest::upsert_fragment_blocks(&mut connection, fragment).await
    }

    pub async fn currently_cached_up_to(&self, chain_key: ChainKey) -> Result<Option<CachedUpTo>> {
        let mut connection = self.pool.get().await?;
        Ok(currently_cached_up_to(&mut connection, chain_key).await)
    }

    pub async fn mark_cached_up_to(&self, chain_key: ChainKey, cached_up_to: H256) -> Result<()> {
        let mut connection = self.pool.get().await?;
        mark_cached_up_to(&mut connection, chain_key, cached_up_to).await
    }

    pub async fn get_highest_attestation_before(
        &self,
        block_number: u64,
        chain_key: ChainKey,
    ) -> Result<Option<attestation::Attestation>> {
        let mut connection = self.pool.get().await?;
        attestation::get_highest_attestation_before(&mut connection, block_number, chain_key).await
    }

    pub async fn get_highest_checkpoint_before(
        &self,
        block_number: u64,
        chain_key: ChainKey,
    ) -> Result<Option<DbCheckpoint>> {
        let mut connection = self.pool.get().await?;
        attestationcheckpoint::get_highest_checkpoint_before(
            &mut connection,
            block_number,
            chain_key,
        )
        .await
    }

    pub async fn get_lowest_attestation_after(
        &self,
        block_number: u64,
        chain_key: ChainKey,
    ) -> Result<Option<attestation::Attestation>> {
        let mut connection = self.pool.get().await?;
        attestation::get_lowest_attestation_after(&mut connection, block_number, chain_key).await
    }

    pub async fn get_lowest_checkpoint_after(
        &self,
        block_number: u64,
        chain_key: ChainKey,
    ) -> Result<Option<DbCheckpoint>> {
        let mut connection = self.pool.get().await?;
        attestationcheckpoint::get_lowest_checkpoint_after(&mut connection, block_number, chain_key)
            .await
    }

    pub async fn get_highest_attestation(
        &self,
        connection: &mut AsyncPgConnection,
        chain_key: ChainKey,
    ) -> Result<Option<attestation::Attestation>> {
        attestation::get_highest_attestation(connection, chain_key).await
    }

    pub async fn get_highest_checkpoint(
        &self,
        connection: &mut AsyncPgConnection,
        chain_key: ChainKey,
    ) -> Result<Option<DbCheckpoint>> {
        attestationcheckpoint::get_highest_checkpoint(connection, chain_key).await
    }
}

pub async fn sync_cache(
    chain_key: ChainKey,
    attestations_cache: &AttestationCacheType,
    cc3_client: &cc3::Client,
    mut historical_sync_rx: UnboundedReceiver<()>,
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
            .start_attestation_sub(attestation_tx, checkpoint_tx, chain_key)
            .await
    });

    // If historical attestations sync isn't yet complete, then we refrain from
    // updating the DB table CachedUpTo. We instead save any update to apply later.
    let mut historical_sync_complete = false;
    let mut cached_up_to: Option<(u64, H256)> = None;

    // Wait on the channels for new attestations and checkpoints
    loop {
        tokio::select! {
            maybe_historical_sync_done = historical_sync_rx.recv() => {
                if let Some(()) = maybe_historical_sync_done {
                    // Mark new height that we are CachedUpTo
                    if let Some(cached_up_to) = cached_up_to {
                        attestations_cache
                        .mark_cached_up_to(cached_up_to.0, cached_up_to.1)
                        .await?;
                    }

                    historical_sync_complete = true;
                }
            },
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
                let Some((checkpoint, chain_key)) = maybe_checkpoint else { break; };

                // check if exists in cache
                if attestations_cache
                    .checkpoint_digest_exists(checkpoint.digest)
                    .await?
                {
                    warn!("Checkpoint already exists in cache, skipping");
                    continue;
                }

                attestations_cache.insert_checkpoint(checkpoint.clone(), chain_key).await?;

                if historical_sync_complete {
                    attestations_cache
                        .mark_cached_up_to(chain_key, checkpoint.digest)
                        .await?;
                } else {
                    cached_up_to = Some((chain_key, checkpoint.digest));
                }
            }
        }
    }

    sync_handle.await??;

    Ok(())
}

/// This process has two main procedures that are quite similar and occur in
/// parallel. We iterate through both attestations and checkpoints from highest
/// block number to lowest. Any attestation or checkpoint missing from the cache
/// is added. The syncing processes end once all attestations and checkpoints have
/// been iterated over.
///
/// Upon the successful conclusion of cache building, the digest of the most recent
/// checkpoint will be recorded in the `CachedUpTo` table. Future cache building
/// passes then stop early when encountering a checkpoint matching that digest.
pub async fn build_historical_cache_for_chain(
    chain: ChainKey,
    attestations_cache: AttestationCacheType,
    cc3_client: CcClientArc,
    done_building_cache: UnboundedSender<()>,
) -> Result<()> {
    info!("Building historical cache for chain: {}", chain);
    let last_digest = cc3_client.fetch_last_digest(chain).await?;

    if let Some(digest) = last_digest {
        // Check for invalid DB state. Any attestations or checkpoints with a higher block number
        // than the latest on-chain block signal that the prover DB has bad state and should be reset.
        let last_block_height = cc3_client
            .get_attestation_by_digest(chain, digest)
            .await?
            .ok_or(anyhow!("Could not get last on-chain attestation"))?
            .header_number();
        let mut connection = attestations_cache.pool.get().await?;
        if let Some(checkpoint) = attestations_cache
            .get_highest_checkpoint(&mut connection, chain)
            .await?
        {
            assert!(
                (from_storage_type(checkpoint.block_number) <= last_block_height),
                "Prover DB contains invalid checkpoint state. Clean DB then run prover to resync."
            );
        }
        if let Some(attestation) = attestations_cache
            .get_highest_attestation(&mut connection, chain)
            .await?
        {
            assert!(
                (from_storage_type(attestation.header_number) <= last_block_height),
                "Prover DB contains invalid attestation state. Clean DB then run prover to resync"
            );
        }

        info!("Starting to sync from: {:?}", digest);
        digest
    } else {
        warn!("No historical attestations found for chain: {}", chain);
        done_building_cache.send(())?;
        return Ok(());
    };

    let client_clone = cc3_client.clone();
    let cache_clone = attestations_cache.clone();
    let attestations_handle = tokio::spawn(async move {
        info!("Spawned task for caching historical attestations",);
        if let Err(e) = cache_historical_attestations(client_clone, cache_clone, chain).await {
            panic!("Error caching historical attestations: {e:?}");
        }
    });

    let checkpoints_handle = tokio::spawn(async move {
        info!("Spawned task for caching historical checkpoints",);
        if let Err(e) = cache_historical_checkpoints(cc3_client, attestations_cache, chain).await {
            panic!("Error caching historical checkpoints: {e:?}");
        }
    });

    let (attestations_result, checkpoints_result) =
        tokio::join!(attestations_handle, checkpoints_handle);
    if let Err(e) = attestations_result {
        panic!("Caching historical attestations join error: {e}");
    }
    if let Err(e) = checkpoints_result {
        panic!("Caching historical checkpoints join error: {e}");
    }

    info!("Finished building historical cache for chain: {}", chain);

    done_building_cache.send(())?;

    Ok(())
}

async fn cache_historical_attestations(
    cc3_client: CcClientArc,
    attestations_cache: AttestationCacheType,
    chain_key: ChainKey,
) -> Result<()> {
    let attestations = cc3_client.get_attestations_for_chain(chain_key).await?;
    for attestation in attestations {
        // Check if the attestation already exists in the cache
        let exists_in_cache = attestations_cache
            .attestation_digest_exists(attestation.attestation.digest())
            .await?;

        if !exists_in_cache {
            // Insert the attestation into the cache
            info!(
                "Inserting attestation with digest({:?}) for chain key: {}, blocknumber: {} into cache",
                attestation.attestation.digest(),
                attestation.chain_key(),
                attestation.header_number(),
            );
            attestations_cache.insert_attestation(attestation).await?;
        }
    }

    Ok(())
}

async fn cache_historical_checkpoints(
    cc3_client: CcClientArc,
    attestations_cache: AttestationCacheType,
    chain_key: ChainKey,
) -> Result<()> {
    let checkpoints = cc3_client.get_checkpoints_for_chain(chain_key).await?;
    // Checkpoint with highest block number will be used to mark
    // the point up to which our cache is complete.
    let highest_checkpoint = if let Some(checkpoint) = checkpoints.first() {
        checkpoint.clone()
    } else {
        info!("No historical checkpoints to cache.");
        return Ok(());
    };

    // All checkpoints prior to this one don't need to be cached. We already have them!
    let cached_up_to = attestations_cache.currently_cached_up_to(chain_key).await?;

    for checkpoint in checkpoints {
        if Some((to_storage_type(chain_key), checkpoint.digest).into()) == cached_up_to {
            info!(
                "Current digest matches the last digest up to which we have already cached all checkpoints {}. Stopping fetching more historical checkpoints",
                checkpoint.digest
            );
            attestations_cache
                .mark_cached_up_to(chain_key, highest_checkpoint.digest)
                .await?;
            return Ok(());
        }

        // Check if checkpoint is already cached
        let exists_in_cache = attestations_cache
            .checkpoint_digest_exists(checkpoint.digest)
            .await?;

        // Insert checkpoint into cache if not present
        if !exists_in_cache {
            info!(
                "Inserting checkpoint with digest({}) for chain: {}, blocknumber: {} into cache",
                checkpoint.digest, chain_key, checkpoint.block_number,
            );
            attestations_cache
                .insert_checkpoint(checkpoint, chain_key)
                .await?;
        }
    }

    info!("Reached the front of the chain, stopping fetching more historical checkpoints");
    attestations_cache
        .mark_cached_up_to(chain_key, highest_checkpoint.digest)
        .await?;

    Ok(())
}
