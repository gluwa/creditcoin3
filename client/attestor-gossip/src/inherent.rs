use anyhow::Result;
use attestor_primitives::{api::AttestorApi, Digest, SignedAttestation};
use attestor_primitives::{InherentError, INHERENT_IDENTIFIER};
use log::{error, info};
use parity_scale_codec::{Codec, Encode};
use sc_client_api::{Backend, HeaderBackend};
use sp_api::ProvideRuntimeApi;
use sp_core::Decode;
use sp_core::H256;
use sp_inherents::{Error, InherentData, InherentIdentifier};
use sp_runtime::traits::Block as BlockT;
use std::sync::{Arc, Mutex};

use crate::{HashFor, LOG_TARGET};

#[derive(Clone)]
pub struct Provider<A, B: BlockT, RA, BE> {
    pub attestation_queue: Vec<SignedAttestation<HashFor<B>, A>>,

    /// runtime api access
    pub runtime_api: Arc<RA>,
    /// Client Backend
    pub backend: Arc<BE>,
}

impl<A, B, RA, BE> Provider<A, B, RA, BE>
where
    B: BlockT,
    RA: ProvideRuntimeApi<B> + Send + Sync + 'static,
    RA::Api: AttestorApi<B, A>,
    BE: Backend<B> + 'static,
    A: Clone + Codec,
{
    pub fn new(backend: Arc<BE>, runtime_api: Arc<RA>) -> Self {
        Self {
            attestation_queue: vec![],
            runtime_api,
            backend,
        }
    }

    // Create multiple copies of this attestation to ensure inclusion
    pub fn create(&mut self, attestation: SignedAttestation<HashFor<B>, A>) -> Result<()> {
        self.attestation_queue.push(attestation);

        Ok(())
    }

    pub fn get(&mut self) -> Option<SignedAttestation<HashFor<B>, A>> {
        self.attestation_queue.pop()
    }

    // Provide a reference to the most recent attestation or return an error
    pub fn get_latest(&self) -> Option<&SignedAttestation<HashFor<B>, A>> {
        self.attestation_queue.last()
    }

    pub fn remove_by_digest(&mut self, digest: Digest) {
        self.attestation_queue
            .retain(|attestation| attestation.digest != digest);
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
    RA::Api: AttestorApi<B, A>,
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
    RA::Api: AttestorApi<B, A>,
    BE: Backend<B> + 'static,
    A: Send + Sync + Encode + Clone + Codec,
{
    async fn provide_inherent_data(
        &self,
        inherent_data: &mut InherentData,
    ) -> Result<(), sp_inherents::Error> {
        info!(target: LOG_TARGET, "📝 Calling attestor inherent provider");

        // Retrieve the latest attestation if available
        let mut provider = self.0.lock().unwrap();
        let block_hash = provider.backend.blockchain().info().best_hash;

        // Get the latest attestation
        let attestation = match provider.get_latest() {
            Some(attestation) => {
                // Get the runtime and fetch te last digest
                let runtime = provider.runtime_api.runtime_api();
                let last_digest = runtime
                    .last_digest(block_hash, attestation.attestation_data.chain_id)
                    .map_err(|_e| sp_inherents::Error::FatalErrorReported)?
                    .unwrap_or(H256::zero());

                // If the last digest matches the one we want to submit in the inherent
                // we can safely remove this pending attestation. It means this was already submitted to the network
                if last_digest == attestation.digest {
                    info!(target: LOG_TARGET, "📝 Attestation inherent with digest {:?} already included on chain, skipping", last_digest);

                    // Attestation with digest already included on chain
                    provider.remove_by_digest(last_digest);

                    return Ok(());
                }

                attestation
            }
            None => return Ok(()), // No attestation available, skip adding data
        };

        // Put data into inherent data
        inherent_data.put_data(INHERENT_IDENTIFIER, &attestation)?;

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
            // prune attestation with digest from provider data so it doesnt get resubmitted
            let mut provider = self.0.lock().unwrap();
            provider.remove_by_digest(digest);
        };

        error!(target: LOG_TARGET, "📝 Get inherent error: {:?}", error);

        Some(Err(Error::Application(Box::from(format!("{:?}", error)))))
    }
}
