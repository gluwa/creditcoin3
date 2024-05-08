use anyhow::Result;
use attestor_primitives::SignedAttestation;
use attestor_primitives::{InherentError, INHERENT_IDENTIFIER};
use log::info;
use parity_scale_codec::Encode;
use sp_core::Decode;
use sp_inherents::{Error, InherentData, InherentIdentifier};

use crate::LOG_TARGET;

pub struct Provider<H, A> {
    pub attestation_queue: Vec<SignedAttestation<H, A>>,
}

impl<H, A> Default for Provider<H, A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<H, A> Provider<H, A> {
    pub fn new() -> Self {
        Self {
            attestation_queue: vec![],
        }
    }

    pub fn create(&mut self, attestation: SignedAttestation<H, A>) -> Result<()> {
        self.attestation_queue.push(attestation);

        Ok(())
    }

    pub fn get(&mut self) -> Option<SignedAttestation<H, A>> {
        self.attestation_queue.pop()
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
