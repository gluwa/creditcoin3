use anyhow::Result;
use attestor_primitives::{api::AttestorApi, Digest, PalletDigest, SignedAttestation};
use attestor_primitives::{InherentError, INHERENT_IDENTIFIER};
use log::{error, info};
use parity_scale_codec::{Codec, Encode};
use sc_client_api::{Backend, HeaderBackend};
use sp_api::ProvideRuntimeApi;
use sp_core::Decode;
use sp_inherents::{Error, InherentData, InherentIdentifier};
use sp_runtime::traits::Block as BlockT;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::{HashFor, LOG_TARGET};

pub const ATTESTATIONS_PER_BLOCK: usize = 10;

#[derive(Clone)]
pub struct Provider<A, B: BlockT, RA, BE> {
    /// attestation queue
    pub attestation_queue: VecDeque<SignedAttestation<HashFor<B>, A>>,
    /// runtime api access
    pub runtime_api: Arc<RA>,
    /// Client Backend
    pub backend: Arc<BE>,
}

impl<A, B, RA, BE> Provider<A, B, RA, BE>
where
    B: BlockT,
    RA: ProvideRuntimeApi<B> + Send + Sync + 'static,
    RA::Api: AttestorApi<B, HashFor<B>, A>,
    BE: Backend<B> + 'static,
    A: Clone + Codec + PartialEq,
{
    pub fn new(backend: Arc<BE>, runtime_api: Arc<RA>) -> Self {
        Self {
            attestation_queue: VecDeque::new(),
            runtime_api,
            backend,
        }
    }

    /// Create a new attestation
    pub fn create(
        &mut self,
        attestation: SignedAttestation<HashFor<B>, A>,
    ) -> Result<(), InherentError> {
        if self.attestation_queue.contains(&attestation) {
            return Err(InherentError::NotValid);
        }

        self.attestation_queue.push_back(attestation);

        Ok(())
    }

    /// Remove an attestation from the queue by digest
    pub fn remove_by_digest(&mut self, digest: Digest) {
        self.attestation_queue
            .retain(|attestation| attestation.attestation.digest() != digest);
    }

    /// Pop the first `x` valid attestations from the queue, skipping invalid ones
    pub fn pop_valid_front_x(&mut self, x: usize) -> Vec<SignedAttestation<HashFor<B>, A>> {
        let block_hash = self.backend.blockchain().info().best_hash;
        let runtime = self.runtime_api.runtime_api();
        let mut valid_attestations = Vec::new();

        while valid_attestations.len() < x {
            match self.attestation_queue.pop_front() {
                Some(att) => {
                    let is_valid =
                        match runtime.contains_digest(block_hash, att.chain_key(), att.digest()) {
                            Ok(exists) => !exists,
                            Err(_) => false, // Treat errors as invalid
                        };

                    if is_valid {
                        valid_attestations.push(att);
                    } else {
                        info!(target: LOG_TARGET, "❌ Discarding invalid attestation: round({:?})", att.round());
                    }
                }
                None => break, // Stop if the queue is empty
            }
        }

        // Sort valid attestations by `block_number`
        valid_attestations.sort_by_key(|a| a.header_number());

        valid_attestations
    }
}

pub struct AsyncProvider<A, B: BlockT, RA, BE>(pub Arc<Mutex<Provider<A, B, RA, BE>>>);

impl<A, B: BlockT, RA, BE> Clone for AsyncProvider<A, B, RA, BE> {
    fn clone(&self) -> Self {
        AsyncProvider(self.0.clone())
    }
}

impl<A, B: BlockT, RA, BE> AsyncProvider<A, B, RA, BE>
where
    B: BlockT,
    RA: ProvideRuntimeApi<B> + Send + Sync + 'static,
    RA::Api: AttestorApi<B, HashFor<B>, A>,
    BE: Backend<B> + 'static,
    A: Clone + Codec + PartialEq,
{
    pub fn new(backend: Arc<BE>, runtime_api: Arc<RA>) -> Self {
        AsyncProvider(Arc::new(Mutex::new(Provider::new(backend, runtime_api))))
    }
}

#[async_trait::async_trait]
impl<A, B, RA, BE> sp_inherents::InherentDataProvider for AsyncProvider<A, B, RA, BE>
where
    B: BlockT,
    RA: ProvideRuntimeApi<B> + Send + Sync + 'static,
    RA::Api: AttestorApi<B, HashFor<B>, A>,
    BE: Backend<B> + 'static,
    A: Send + Sync + Encode + Clone + Codec + PartialEq,
{
    async fn provide_inherent_data(
        &self,
        inherent_data: &mut InherentData,
    ) -> Result<(), sp_inherents::Error> {
        info!(target: LOG_TARGET, "📝 Calling attestor inherent provider");

        // Retrieve the latest attestation if available
        let mut provider = self.0.lock().map_err(|e| {
            error!("error acquiring attestation inherent provider lock {:?}", e);
            sp_inherents::Error::FatalErrorReported
        })?;

        // Get max 10 attestations from the provider
        let attestations = provider.pop_valid_front_x(ATTESTATIONS_PER_BLOCK);
        inherent_data.put_data(INHERENT_IDENTIFIER, &attestations)?;

        Ok(())
    }

    async fn try_handle_error(
        &self,
        identifier: &InherentIdentifier,
        mut error: &[u8],
    ) -> Option<Result<(), Error>> {
        if *identifier != INHERENT_IDENTIFIER {
            return None;
        }

        let error = InherentError::decode(&mut error).ok()?;

        if let InherentError::Duplicate(digest) = error {
            info!(target: LOG_TARGET, "📝 Attestation inherent with digest {:?} already included on chain, skipping (in try handle error)", digest);
            // prune attestation with digest from provider data so it doesnt get resubmitted
            let mut provider = self.0.lock().unwrap();
            provider.remove_by_digest(digest);
        };

        error!(target: LOG_TARGET, "📝 Get inherent error: {:?}", error);

        Some(Err(Error::Application(Box::from(format!("{:?}", error)))))
    }
}
