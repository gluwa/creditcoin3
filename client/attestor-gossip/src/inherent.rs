use anyhow::Result;
use attestor_primitives::SignedAttestation;
use attestor_primitives::{InherentError, INHERENT_IDENTIFIER};
use parity_scale_codec::Encode;
use sp_core::Decode;
use sp_inherents::{Error, InherentData, InherentIdentifier};

pub struct Provider<H> {
    pub attestation_queue: Vec<SignedAttestation<H>>,
}

impl<H> Default for Provider<H> {
    fn default() -> Self {
        Self::new()
    }
}

impl<H> Provider<H> {
    pub fn new() -> Self {
        Self {
            attestation_queue: vec![],
        }
    }

    pub fn create(&mut self, attestation: SignedAttestation<H>) -> Result<()> {
        self.attestation_queue.push(attestation);

        Ok(())
    }

    pub fn get(&mut self) -> Option<SignedAttestation<H>> {
        self.attestation_queue.pop()
    }
}

pub struct AttestationInherent<H> {
    pub attestation: Option<SignedAttestation<H>>,
}

impl<H> AttestationInherent<H> {
    pub fn new(attestation: Option<SignedAttestation<H>>) -> Self {
        Self { attestation }
    }
}

#[async_trait::async_trait]
impl<H> sp_inherents::InherentDataProvider for AttestationInherent<H>
where
    H: Send + Sync + Encode,
{
    async fn provide_inherent_data(&self, inherent_data: &mut InherentData) -> Result<(), Error> {
        log::info!("CALLING GOSSIP INHERENT PROVIDER");

        if let Some(_attestation) = &self.attestation {
            log::info!("GOT ATTESTATION TO SUBMIT");

            inherent_data.put_data(INHERENT_IDENTIFIER, &_attestation)
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
