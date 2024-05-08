use anyhow::Result;
use attestor_primitives::{Digest, SignedAttestation};
use attestor_primitives::{InherentError, INHERENT_IDENTIFIER};
use log::{error, info};
use parity_scale_codec::Encode;
use sp_core::Decode;
use sp_inherents::{Error, InherentData, InherentIdentifier};
use std::sync::{Arc, Mutex};

use crate::LOG_TARGET;

#[derive(Clone)]
pub struct Provider<H, A> {
    pub attestation_queue: Vec<SignedAttestation<H, A>>,
}

impl<H, A> Default for Provider<H, A>
where
    A: Clone,
    H: Clone,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<H, A> Provider<H, A>
where
    A: Clone,
    H: Clone,
{
    pub fn new() -> Self {
        Self {
            attestation_queue: vec![],
        }
    }

    // Create multiple copies of this attestation to ensure inclusion
    pub fn create(&mut self, attestation: SignedAttestation<H, A>) -> Result<()> {
        self.attestation_queue.push(attestation.clone());
        self.attestation_queue.push(attestation.clone());
        self.attestation_queue.push(attestation);

        Ok(())
    }

    pub fn get(&mut self) -> Option<SignedAttestation<H, A>> {
        self.attestation_queue.pop()
    }

    // Provide a reference to the most recent attestation or return an error
    pub fn get_latest(&self) -> Option<&SignedAttestation<H, A>> {
        self.attestation_queue.last()
    }

    pub fn remove_by_digest(&mut self, digest: Digest)
    where
        H: PartialEq<Digest>,
    {
        self.attestation_queue
            .retain(|attestation| attestation.digest != digest);
    }
}

pub struct AsyncProvider<H, A>(pub Arc<Mutex<Provider<H, A>>>);

impl<H, A> Default for AsyncProvider<H, A>
where
    A: Clone,
    H: Clone,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<H, A> Clone for AsyncProvider<H, A> {
    fn clone(&self) -> Self {
        AsyncProvider(self.0.clone())
    }
}

impl<H, A> AsyncProvider<H, A>
where
    A: Clone,
    H: Clone,
{
    pub fn new() -> Self {
        AsyncProvider(Arc::new(Mutex::new(Provider::default())))
    }
}

#[async_trait::async_trait]
impl<H, A> sp_inherents::InherentDataProvider for AsyncProvider<H, A>
where
    H: Send + Sync + Encode + Clone + std::cmp::PartialEq<sp_core::H256>,
    A: Send + Sync + Encode + Clone,
{
    async fn provide_inherent_data(
        &self,
        inherent_data: &mut InherentData,
    ) -> Result<(), sp_inherents::Error> {
        info!(target: LOG_TARGET, "📝 Calling attestor inherent provider");

        // Retrieve the latest attestation if available
        let lock = self.0.lock().unwrap();
        let attestation = match lock.get_latest() {
            Some(attestation) => attestation,
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
            let mut lock = self.0.lock().unwrap();
            lock.remove_by_digest(digest);
        };

        error!(target: LOG_TARGET, "📝 Get inherent error: {:?}", error);

        Some(Err(Error::Application(Box::from(format!("{:?}", error)))))
    }
}

pub struct AttestationInherent<H, A> {
    pub attestation: Option<SignedAttestation<H, A>>,
}

impl<H, A> AttestationInherent<H, A> {
    pub fn new(attestation: Option<SignedAttestation<H, A>>) -> Self {
        Self { attestation }
    }
}

#[async_trait::async_trait]
impl<H, A> sp_inherents::InherentDataProvider for AttestationInherent<H, A>
where
    H: Send + Sync + Encode,
    A: Send + Sync + Encode,
{
    async fn provide_inherent_data(&self, inherent_data: &mut InherentData) -> Result<(), Error> {
        info!(target: LOG_TARGET, "📝 Calling attestor inherent provider");

        if let Some(attestation) = &self.attestation {
            info!(target: LOG_TARGET, "📝 Got an attestation inherent to submit");

            inherent_data.put_data(INHERENT_IDENTIFIER, &attestation)
        } else {
            Ok(())
        }
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

        Some(Err(Error::Application(Box::from(format!("{:?}", error)))))
    }
}
