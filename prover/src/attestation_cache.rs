use anyhow::Result;
use attestor_primitives::{ChainId, Digest, SignedAttestation};
use hex::ToHex;
use std::marker::PhantomData;

use crate::postgres::{
    attestation::{self, Attestation},
    db::PgPool,
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
    pub async fn get_by_digest(&self, digest: Digest) -> Result<Attestation> {
        let mut connection = self.pool.get().await?;
        let attestation = attestation::get_by_digest(&mut connection, digest.encode_hex()).await?;

        Ok(attestation)
    }

    pub async fn digest_exists(&self, digest: Digest) -> Result<bool> {
        let mut connection = self.pool.get().await?;
        attestation::exists_by_digest(&mut connection, digest.encode_hex()).await
    }

    pub async fn insert(&self, attestation: SignedAttestation<H, A>) -> Result<()> {
        let mut connection = self.pool.get().await?;
        attestation::insert(&mut connection, attestation.into()).await?;

        Ok(())
    }

    pub async fn first_digest_exists(&self, chain_id: ChainId) -> Result<bool> {
        let mut connection = self.pool.get().await?;
        attestation::first_digest_exists(&mut connection, chain_id).await
    }

    pub async fn last_synced_attestation(&self, chain_id: ChainId) -> Result<Option<Attestation>> {
        let mut connection = self.pool.get().await?;
        attestation::last_synced(&mut connection, chain_id).await
    }
}
