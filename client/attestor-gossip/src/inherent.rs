use anyhow::Result;
use attestor_primitives::{api::AttestorApi, Digest, SignedAttestation};
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
    A: Clone + Codec,
{
    pub fn new(backend: Arc<BE>, runtime_api: Arc<RA>) -> Self {
        Self {
            attestation_queue: VecDeque::new(),
            runtime_api,
            backend,
        }
    }

    // Create an attestation adds it to the queue
    pub fn create(&mut self, attestation: SignedAttestation<HashFor<B>, A>) -> Result<()> {
        self.attestation_queue.push_back(attestation);

        Ok(())
    }

    // Provide a reference to the most recent attestation
    pub fn get_latest(&mut self) -> Option<SignedAttestation<HashFor<B>, A>> {
        self.attestation_queue.pop_front()
    }

    // Removes an attestation from the queue by digest
    pub fn remove_by_digest(&mut self, digest: Digest) {
        self.attestation_queue
            .retain(|attestation| attestation.attestation.digest() != digest);
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
    A: Clone + Codec,
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
    A: Send + Sync + Encode + Clone + Codec,
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

        let block_hash = provider.backend.blockchain().info().best_hash;

        // Get the latest attestation
        while let Some(attestation) = provider.get_latest() {
            // Get the runtime and fetch the last digest
            let runtime = provider.runtime_api.runtime_api();

            let digest = attestation.attestation.digest();

            let contains_digest = runtime
                .contains_digest(block_hash, attestation.attestation.chain_id, digest)
                .map_err(|_e| sp_inherents::Error::FatalErrorReported)?;

            // If the last digest matches the one we want to submit in the inherent
            // we can safely remove this pending attestation. It means this was already submitted to the network
            // Check if the attestation is already included on the chain
            if contains_digest {
                info!(target: LOG_TARGET, "📝 Attestation inherent with digest {:?} already included on chain, skipping", digest);
                provider.remove_by_digest(digest);
            } else {
                // Update inherent data and then remove the attestation from queue
                inherent_data.put_data(INHERENT_IDENTIFIER, &attestation)?;
                info!(target: LOG_TARGET, "📝 Attestation inherent with digest {:?} submitted", digest);
                provider.remove_by_digest(digest);
                break; // Break the loop since we successfully submitted an attestation
            }
        }

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
