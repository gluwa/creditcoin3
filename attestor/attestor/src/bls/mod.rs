mod error;

pub use error::Error;
use user::prelude::*;

/// Thread safe lazily evaluated attestor BLS pubkey store.
///
/// BLS pubkeys are meant to be updated only on attestor set rotation. This keeps runtime fetches to
/// a minimum for sections of the code with frequent access to bls keys, such as during gossipsub
/// message validation in the [p2p worker].
///
/// [p2p worker]: crate::worker::p2p
pub struct BlsStore {
    chain_key: attestor_primitives::ChainKey,
    keys: tokio::sync::Mutex<std::collections::HashMap<[u8; 32], bls_signatures::PublicKey>>,
}

impl BlsStore {
    pub async fn new(
        cc3: &mut cc_client::Client,
        chain_key: attestor_primitives::ChainKey,
        attestors: &[cc_client::AccountId32],
    ) -> Result<Self, Interrupt<Error>> {
        Ok(Self {
            chain_key,
            keys: tokio::sync::Mutex::new(keys(cc3, chain_key, attestors).await?),
        })
    }

    pub async fn note_attestors_elected(
        &self,
        cc3: &mut cc_client::Client,
        attestors: &[cc_client::AccountId32],
    ) -> Result<(), Interrupt<Error>> {
        // NOTE: bls keys are updated atomically only if the new keys could be fetched
        // successfully.
        let _ = std::mem::replace(
            &mut *self.keys.lock().await,
            keys(cc3, self.chain_key, attestors).await?,
        );

        Ok(())
    }

    pub async fn pubkey(
        &self,
        attestor: impl AsRef<[u8; 32]>,
    ) -> Option<bls_signatures::PublicKey> {
        self.keys.lock().await.get(attestor.as_ref()).cloned()
    }
}

async fn keys(
    cc3: &mut cc_client::Client,
    chain_key: attestor_primitives::ChainKey,
    attestors: &[cc_client::AccountId32],
) -> Result<std::collections::HashMap<[u8; 32], bls_signatures::PublicKey>, Interrupt<Error>> {
    use bls_signatures::Serialize as _;

    let mut keys_new = std::collections::HashMap::with_capacity(attestors.len());
    let mut runtime_api = cc_client::api::ReconnectingRuntimeApi::new(cc3)
        .await
        .map_interrupt(Error::Client)?;

    for attestor_id in attestors {
        let call = || {
            let attestor: &[u8; 32] = attestor_id.as_ref();
            cc_client::Client::runtime_api()
                .attestor_api()
                .attestor_bls_pubkey(chain_key, (*attestor).into())
        };

        match runtime_api
            .call(call)
            .await
            .map_interrupt(Error::Client)?
            .map(|bytes| bls_signatures::PublicKey::from_bytes(&bytes))
        {
            Some(Ok(pubkey)) => keys_new.insert(*attestor_id.as_ref(), pubkey),
            Some(Err(..)) => {
                return Err(Interrupt::Cont(Error::InvalidBls(attestor_id.clone())));
            }
            None => {
                return Err(Interrupt::Cont(Error::Unregistered(attestor_id.clone())));
            }
        };
    }

    Ok(keys_new)
}
