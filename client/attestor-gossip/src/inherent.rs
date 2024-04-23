use anyhow::Result;
use attestor_primitives::{InherentError, INHERENT_IDENTIFIER};
use sp_core::Decode;
use sp_inherents::{Error, InherentData, InherentIdentifier};

use crate::Attestation;

pub struct Provider<B> {
    pub attestation_queue: Vec<Attestation<B>>,
}

impl<B> Provider<B> {
    pub fn new() -> Self {
        Self {
            attestation_queue: vec![],
        }
    }

    pub fn create(&mut self, attestation: Attestation<B>) -> Result<()> {
        self.attestation_queue.push(attestation);

        Ok(())
    }

    pub fn get(&mut self) -> Option<Attestation<B>> {
        self.attestation_queue.pop()
    }
}

pub struct AttestationInherent<B> {
    pub attestation: Option<Attestation<B>>,
}

impl<B> AttestationInherent<B> {
    pub fn new(attestation: Option<Attestation<B>>) -> Self {
        Self { attestation }
    }
}

#[async_trait::async_trait]
impl<B> sp_inherents::InherentDataProvider for AttestationInherent<B>
where
    B: Send + Sync,
{
    async fn provide_inherent_data(&self, inherent_data: &mut InherentData) -> Result<(), Error> {
        log::info!("CALLING GOSSIP INHERENT PROVIDER");

        if let Some(_attestation) = &self.attestation {
            inherent_data.put_data(INHERENT_IDENTIFIER, &_attestation.attestor)
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
