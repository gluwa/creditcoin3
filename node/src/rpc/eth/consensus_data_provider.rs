use std::sync::Arc;

use fc_rpc::pending::ConsensusDataProvider;
use sc_client_api::{AuxStore, UsageProvider};
use sc_consensus_babe::{
    authorship::claim_slot, AuthorityId, BabeAuthorityWeight, BabeConfiguration,
    CompatibleDigestItem, Epoch, NextEpochDescriptor, PreDigest, SecondaryPlainPreDigest,
};
use sc_consensus_epochs::{descendent_query, SharedEpochChanges, ViableEpochDescriptor};
use scale_codec::Encode;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::{HeaderBackend, HeaderMetadata};
use sp_consensus_babe::{inherents::BabeInherentData, BabeApi, ConsensusLog, Slot, BABE_ENGINE_ID};
use sp_keystore::KeystorePtr;
use sp_runtime::{
    traits::{Block as BlockT, Header},
    DigestItem,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("BABE inherent data missing")]
    MissingInherent,

    #[error("Failed to get BABE config")]
    MissingConfig(#[source] sp_blockchain::Error),

    #[error("Failed to get epoch descriptor: {0}")]
    EpochDataQuery(String),

    #[error("Failed to get viable epoch")]
    NoViableEpoch,

    #[error("Consensus error: {0}")]
    Consensus(#[from] sp_consensus::Error),

    #[error("{0}")]
    Other(String),
}

impl From<Error> for sp_inherents::Error {
    fn from(err: Error) -> Self {
        sp_inherents::Error::Application(Box::new(err))
    }
}

impl From<Error> for sc_service::Error {
    fn from(err: Error) -> Self {
        sc_service::Error::Application(Box::new(err))
    }
}

fn application_error(err: impl std::error::Error + Send + Sync + 'static) -> sp_inherents::Error {
    sp_inherents::Error::Application(Box::new(err))
}

pub struct BabeConsensusDataProvider<B: BlockT, C> {
    /// shared reference to keystore
    keystore: KeystorePtr,

    /// Shared reference to the client.
    client: Arc<C>,

    /// Shared epoch changes
    epoch_changes: SharedEpochChanges<B, Epoch>,

    /// BABE config, gotten from the runtime.
    /// NOTE: This is used to fetch `slot_duration` and `epoch_length` in the
    /// `ConsensusDataProvider` implementation. Correct as far as these values
    /// are not changed during an epoch change.
    config: BabeConfiguration,

    /// Authorities to be used for this babe chain.
    authorities: Vec<(AuthorityId, BabeAuthorityWeight)>,
}

impl<B, C> BabeConsensusDataProvider<B, C>
where
    B: BlockT,
    C: AuxStore
        + ProvideRuntimeApi<B>
        + UsageProvider<B>
        + HeaderBackend<B>
        + HeaderMetadata<B, Error = sp_blockchain::Error>,
    C::Api: BabeApi<B>,
{
    pub fn new(
        client: Arc<C>,
        keystore: KeystorePtr,
        epoch_changes: SharedEpochChanges<B, Epoch>,
        authorities: Vec<(AuthorityId, BabeAuthorityWeight)>,
    ) -> Result<Self, Error> {
        let config =
            sc_consensus_babe::configuration(&*client).map_err(|e| Error::MissingConfig(e))?;

        Ok(Self {
            client,
            epoch_changes,
            authorities,
            keystore,
            config,
        })
    }

    fn epoch(&self, parent: &<B as BlockT>::Header, slot: Slot) -> Result<Epoch, Error> {
        let epoch_changes = self.epoch_changes.shared_data();
        let epoch_descriptor = epoch_changes
            .epoch_descriptor_for_child_of(
                descendent_query(&*self.client),
                &parent.hash(),
                *parent.number(),
                slot,
            )
            .map_err(|e| Error::EpochDataQuery(e.to_string()))?
            .ok_or(Error::Consensus(sp_consensus::Error::InvalidAuthoritiesSet))?;

        let epoch = epoch_changes
            .viable_epoch(&epoch_descriptor, |slot| Epoch::genesis(&self.config, slot))
            .ok_or(Error::NoViableEpoch)?;

        Ok(epoch.as_ref().clone())
    }
}

impl<B, C> ConsensusDataProvider<B> for BabeConsensusDataProvider<B, C>
where
    B: BlockT,
    C: AuxStore
        + ProvideRuntimeApi<B>
        + UsageProvider<B>
        + HeaderBackend<B>
        + HeaderMetadata<B, Error = sp_blockchain::Error>,
    C::Api: BabeApi<B>,
{
    fn create_digest(
        &self,
        parent: &<B as BlockT>::Header,
        data: &sp_inherents::InherentData,
    ) -> Result<sp_runtime::Digest, sp_inherents::Error> {
        let slot = data
            .babe_inherent_data()?
            .ok_or(sp_inherents::Error::Application(Box::new(
                Error::MissingInherent,
            )))?;

        let epoch = self
            .epoch(parent, slot)
            .map_err(|e| sp_inherents::Error::Application(Box::new(e)))?;

        let logs =
            if let Some((pre_digest, _authority_id)) = claim_slot(slot, &epoch, &self.keystore) {
                vec![<DigestItem as CompatibleDigestItem>::babe_pre_digest(
                    pre_digest,
                )]
            } else {
                // well we couldn't claim a slot because this is an existing chain and we're not in the
                // authorities. we need to tell BabeBlockImport that the epoch has changed, and we put
                // ourselves in the authorities.
                let predigest = PreDigest::SecondaryPlain(SecondaryPlainPreDigest {
                    slot,
                    authority_index: 0_u32,
                });

                let mut epoch_changes = self.epoch_changes.shared_data();
                let epoch_descriptor = epoch_changes
                    .epoch_descriptor_for_child_of(
                        descendent_query(&*self.client),
                        &parent.hash(),
                        *parent.number(),
                        slot,
                    )
                    .map_err(|e| Error::Other(format!("failed to fetch epoch_descriptor: {}", e)))?
                    .ok_or(application_error(
                        sp_consensus::Error::InvalidAuthoritiesSet,
                    ))?;

                match epoch_descriptor {
                    ViableEpochDescriptor::Signaled(identifier, _epoch_header) => {
                        let epoch_mut = epoch_changes.epoch_mut(&identifier).ok_or(
                            sp_inherents::Error::Application(Box::new(
                                sp_consensus::Error::InvalidAuthoritiesSet,
                            )),
                        )?;

                        // mutate the current epoch
                        epoch_mut.authorities = self.authorities.clone();

                        let next_epoch = ConsensusLog::NextEpochData(NextEpochDescriptor {
                            authorities: self.authorities.clone(),
                            // copy the old randomness
                            randomness: epoch_mut.randomness,
                        });

                        vec![
                            DigestItem::PreRuntime(BABE_ENGINE_ID, predigest.encode()),
                            DigestItem::Consensus(BABE_ENGINE_ID, next_epoch.encode()),
                        ]
                    }
                    ViableEpochDescriptor::UnimportedGenesis(_) => {
                        // since this is the genesis, secondary predigest works for now.
                        vec![DigestItem::PreRuntime(BABE_ENGINE_ID, predigest.encode())]
                    }
                }
            };

        Ok(sp_runtime::Digest { logs })
    }
}
