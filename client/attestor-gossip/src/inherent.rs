use anyhow::Result;
use attestor_primitives::{AttestationInherentData, InherentError, INHERENT_IDENTIFIER};
use sp_core::Decode;
use sp_inherents::{Error, InherentData, InherentIdentifier};

pub struct Provider {
    pub attestation_queue: Vec<AttestationInherentData>,
}

impl Provider {
    pub fn new() -> Self {
        Self {
            attestation_queue: vec![],
        }
    }

    pub fn create(&mut self, attestation: AttestationInherentData) -> Result<()> {
        self.attestation_queue.push(attestation);

        Ok(())
    }

    pub fn get(&mut self) -> Option<AttestationInherentData> {
        self.attestation_queue.pop()
    }
}

pub struct AttestationInherent {
    pub attestation: Option<AttestationInherentData>,
}

impl AttestationInherent {
    pub fn new(attestation: Option<AttestationInherentData>) -> Self {
        Self { attestation }
    }
}

#[async_trait::async_trait]
impl sp_inherents::InherentDataProvider for AttestationInherent {
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
