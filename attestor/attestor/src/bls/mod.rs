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
pub struct BlsStore(tokio::sync::Mutex<BlsStoreInner>);

struct BlsStoreInner {
    chain_key: attestor_primitives::ChainKey,
    keys: std::collections::HashMap<[u8; 32], bls_signatures::PublicKey>,
}

impl BlsStore {
    pub async fn new(
        cc3: &mut cc_client::Client,
        chain_key: attestor_primitives::ChainKey,
        attestors: &[cc_client::AccountId32],
    ) -> Result<Self, Interrupt<Error>> {
        use bls_signatures::Serialize as _;

        let api_calls = cc_client::Client::runtime_api();
        let mut runtime_api = cc_client::api::ReconnectingRuntimeApi::new(cc3)
            .await
            .map_interrupt(Error::Client)?;

        let mut keys = std::collections::HashMap::with_capacity(attestors.len());

        for attestor_id in attestors {
            let call = || {
                let attestor: &[u8; 32] = attestor_id.as_ref();
                api_calls
                    .attestor_api()
                    .attestor_bls_pubkey(chain_key, (*attestor).into())
            };

            match runtime_api
                .call(call)
                .await
                .map_interrupt(Error::Client)?
                .map(|bytes| bls_signatures::PublicKey::from_bytes(&bytes))
            {
                Some(Ok(pubkey)) => keys.insert(*attestor_id.as_ref(), pubkey),
                Some(Err(..)) => {
                    return Err(Interrupt::Cont(Error::InvalidBls(attestor_id.clone())));
                }
                None => {
                    return Err(Interrupt::Cont(Error::Unregistered(attestor_id.clone())));
                }
            };
        }

        Ok(Self(tokio::sync::Mutex::new(BlsStoreInner {
            chain_key,
            keys,
        })))
    }

    pub async fn note_attestors_elected(
        &self,
        cc3: &mut cc_client::Client,
        attestors: &[cc_client::AccountId32],
    ) -> Result<(), Interrupt<Error>> {
        use bls_signatures::Serialize as _;

        let mut inner = self.0.lock().await;
        let mut keys_new = std::collections::HashMap::<[u8; 32], _>::with_capacity(attestors.len());

        let api_calls = cc_client::Client::runtime_api();
        let chain_key = inner.chain_key;

        let mut runtime_api = cc_client::api::ReconnectingRuntimeApi::new(cc3)
            .await
            .map_interrupt(Error::Client)?;

        for attestor_id in attestors {
            let call = || {
                let attestor: &[u8; 32] = attestor_id.as_ref();
                api_calls
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

        // WARNING: only update bls keys if the previous fetch was successful.
        std::mem::swap(&mut inner.keys, &mut keys_new);

        Ok(())
    }

    pub async fn pubkey(
        &self,
        attestor: impl AsRef<[u8; 32]>,
    ) -> Option<bls_signatures::PublicKey> {
        self.0.lock().await.keys.get(attestor.as_ref()).cloned()
    }
}
