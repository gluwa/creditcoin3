use std::str::FromStr;

use attestor_primitives::AttestorId;
use sp_core::{sr25519, Pair};
use subxt::utils::AccountId32;
use subxt_signer::{
    sr25519::{Keypair, Signature},
    SecretUri,
};

#[derive(Clone)]
pub struct CC3Signer {
    pub(crate) signing_keypair: Keypair,
    pub(crate) pair: sr25519::Pair,
}

impl CC3Signer {
    pub fn new(key: &str) -> anyhow::Result<Self> {
        Ok(Self {
            signing_keypair: Keypair::from_uri(&SecretUri::from_str(key)?)?,
            pair: sr25519::Pair::from_string(key, None)?,
        })
    }

    #[must_use]
    pub fn attestor_id(&self) -> AttestorId {
        AttestorId::from_public(self.signing_keypair.public_key().0)
    }

    #[must_use]
    pub fn account_id(&self) -> AccountId32 {
        AccountId32(self.signing_keypair.public_key().0)
    }

    #[must_use]
    pub fn sign(&self, message: &[u8]) -> Signature {
        self.signing_keypair.sign(message)
    }
}
