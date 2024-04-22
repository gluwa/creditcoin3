use attestor_primitives::{InherentError, INHERENT_IDENTIFIER};
use sp_core::Decode;
use sp_inherents::{Error, InherentData, InherentIdentifier};

use crate::Attestation;

/// Provider for inherent data.
pub struct AttestationInherent<B> {
    pub attestation: Attestation<B>,
    pub signatures: Vec<u8>,
}

impl<B> AttestationInherent<B> {
    pub fn new(attestation: Attestation<B>, signatures: Vec<u8>) -> Self {
        AttestationInherent {
            attestation,
            signatures,
        }
    }
}

#[async_trait::async_trait]
impl<B> sp_inherents::InherentDataProvider for AttestationInherent<B>
where
    B: Send + Sync,
{
    async fn provide_inherent_data(&self, inherent_data: &mut InherentData) -> Result<(), Error> {
        inherent_data.put_data(INHERENT_IDENTIFIER, &self.signatures)
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
