use anyhow::{anyhow, Result};
use diesel_async::AsyncPgConnection;
use hex::ToHex;
use sp_core::H256;
use std::marker::PhantomData;
use tracing::{debug, info, warn};

use attestor_primitives::{AttestationCheckpoint, ChainKey, Digest, SignedAttestation};
use cc_client::Client as CcClient;

use crate::postgres::{
    self,
    attestation::{self, Error as DbAttestationError},
    attestationcheckpoint::{
        self, AttestationCheckpoint as DbCheckpoint, Error as DbCheckpointError,
    },
    blockwithdigest,
    cachedupto::{currently_cached_up_to, mark_cached_up_to, CachedUpTo},
    db::PgPool,
    from_storage_type,
    queryfragmenttype::{self, NewQueryFragmentType, QueryFragmentType},
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
        if let Err(e) = attestation::insert(&mut connection, attestation.clone().into()).await {
            match e {
                DbAttestationError::DuplicateDigestPrevDigest => {
                    return Err(
                        anyhow!("Inserted attestation with duplicate (digest: {}, prev_digest: {:?}). Therefore DB contains bad state. Clean DB and run prover to resync. Error: {:?}", attestation.digest(), attestation.prev_digest(), e),
                    );
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
        if let Err(e) = attestationcheckpoint::insert(&mut connection, db_checkpoint.clone()).await
        {
            match e {
                DbCheckpointError::DuplicateDigest => {
                    return Err(anyhow!("Inserted checkpoint with duplicate (digest: {}). Therefore DB contains bad state. Clean DB and run prover to resync. Error: {:?}", db_checkpoint.digest, e));
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

    pub async fn insert_many_checkpoints(
        &self,
        chain_key: ChainKey,
        checkpoints: &[AttestationCheckpoint],
    ) -> Result<()> {
        let mut connection = self.pool.get().await?;
        let db_checkpoints: Vec<DbCheckpoint> = checkpoints
            .iter()
            .map(|c| DbCheckpoint::from_on_chain(c, chain_key))
            .collect();

        attestationcheckpoint::insert_many(&mut connection, &db_checkpoints).await?;

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

    pub async fn last_synced_attestation_block_number(
        &self,
        chain_key: ChainKey,
    ) -> Result<Option<u64>> {
        let mut connection = self.pool.get().await?;
        attestation::last_synced(&mut connection, chain_key)
            .await
            .map(|opt_attestation| {
                opt_attestation.map(|attestation| attestation.header_number as u64)
            })
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

    pub async fn get_query_fragment_type_by_id(
        &self,
        query_id: String,
    ) -> Result<Option<QueryFragmentType>> {
        let mut connection = self.pool.get().await?;
        queryfragmenttype::get_by_query_id(&mut connection, query_id)
            .await
            .map_err(|e| anyhow!("Failed to get query fragment type: {:?}", e))
    }

    pub async fn upsert_query_fragment_type(
        &self,
        new_query_fragment_type: NewQueryFragmentType,
    ) -> Result<()> {
        let mut connection = self.pool.get().await?;
        queryfragmenttype::upsert(&mut connection, new_query_fragment_type)
            .await
            .map_err(|e| anyhow!("Failed to upsert query fragment type: {:?}", e))
    }

    pub async fn query_fragment_type_exists_by_id(&self, query_id: String) -> Result<bool> {
        let mut connection = self.pool.get().await?;
        queryfragmenttype::exists_by_query_id(&mut connection, query_id)
            .await
            .map_err(|e| anyhow!("Failed to check if query fragment type exists: {:?}", e))
    }
}

impl<H, A> AttestationCache<H, A>
where
    H: AsRef<[u8]> + Clone + Copy,
    A: AsRef<[u8]> + Clone,
{
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
        &mut self,
        chain: ChainKey,
        cc3_client: &CcClient,
    ) -> Result<()> {
        debug!("🛠️ Building historical cache for chain: {}", chain);
        let last_digest = cc3_client.fetch_last_digest(chain).await?;

        if let Some(digest) = last_digest {
            // Check for invalid DB state. Any attestations or checkpoints with a higher block number
            // than the latest on-chain block signal that the prover DB has bad state and should be reset.
            let last_block_height = cc3_client
                .get_attestation_by_digest(chain, digest)
                .await?
                .ok_or(anyhow!("Could not get last on-chain attestation"))?
                .header_number();
            let mut connection = self.pool.get().await?;
            if let Some(checkpoint) = self.get_highest_checkpoint(&mut connection, chain).await? {
                assert!(
                (from_storage_type(checkpoint.block_number) <= last_block_height),
                "Prover DB contains invalid checkpoint state. Clean DB then run prover to resync."
            );
            }
            if let Some(attestation) = self.get_highest_attestation(&mut connection, chain).await? {
                assert!(
                (from_storage_type(attestation.header_number) <= last_block_height),
                "Prover DB contains invalid attestation state. Clean DB then run prover to resync"
            );
            }

            debug!("🟢 Starting to sync from: {}", digest);
            digest
        } else {
            warn!("⚠️ No historical attestations found for chain: {}", chain);
            return Ok(());
        };

        self.cache_historical_checkpoints(cc3_client, chain).await?;
        debug!(
            "✅ Finished building historical checkpoints for chain: {}",
            chain
        );
        self.cache_historical_attestations(cc3_client, chain)
            .await?;
        debug!(
            "✅ Finished building historical attestations for chain: {}",
            chain
        );

        debug!("✅ Finished building historical cache for chain: {}", chain);

        Ok(())
    }

    async fn cache_historical_attestations(
        &mut self,
        cc3_client: &CcClient,
        chain_key: ChainKey,
    ) -> Result<()> {
        let attestations = cc3_client.get_attestations_for_chain(chain_key).await?;
        let mut connection = self.pool.get().await?;
        let db_attestations: Vec<attestation::Attestation> = attestations
            .into_iter()
            .map(std::convert::Into::into)
            .collect();

        Ok(attestation::insert_many(&mut connection, db_attestations).await?)
    }

    async fn cache_historical_checkpoints(
        &mut self,
        cc3_client: &CcClient,
        chain_key: ChainKey,
    ) -> Result<()> {
        // Expensive call
        let checkpoints = cc3_client.get_checkpoints_for_chain(chain_key).await?;
        // Checkpoint with highest block number will be used to mark
        // the point up to which our cache is complete.
        let highest_checkpoint = if let Some(checkpoint) = checkpoints.first() {
            debug!(
                "🔍 Found highest checkpoint with block number: {} for chain key: {}",
                checkpoint.block_number, chain_key
            );
            checkpoint.clone()
        } else {
            debug!("📭 No historical checkpoints to cache.");
            return Ok(());
        };

        // Store all checkpoints in the cache
        self.insert_many_checkpoints(chain_key, &checkpoints)
            .await?;

        info!(
            "💾 Cached {} historical checkpoints for chain key: {}",
            checkpoints.len(),
            chain_key
        );
        self.mark_cached_up_to(chain_key, highest_checkpoint.digest)
            .await?;

        Ok(())
    }
}
