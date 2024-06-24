use anyhow::Result;
use attestor_primitives::Digest;
use hex::ToHex;

use crate::postgres::{
    attestation::{self, Attestation},
    db::PgPool,
};

pub struct AttestationCache {
    pool: PgPool,
}

impl AttestationCache {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        AttestationCache { pool }
    }
}

impl AttestationCache {
    pub async fn get_by_digest(&self, digest: Digest) -> Result<Option<Attestation>> {
        let mut connection = self.pool.get().await?;
        let attestation = attestation::get_by_digest(&mut connection, digest.encode_hex()).await?;

        Ok(attestation)
    }

    pub async fn digest_exists(&self, digest: Digest) -> Result<bool> {
        let mut connection = self.pool.get().await?;
        let attestation = attestation::get_by_digest(&mut connection, digest.encode_hex()).await?;

        Ok(attestation.is_some())
    }

    // TODO: accept signed atttestation instead of the db type
    pub async fn insert(&self, attestation: Attestation) -> Result<()> {
        let mut connection = self.pool.get().await?;
        attestation::insert(&mut connection, attestation).await?;

        Ok(())
    }
}
