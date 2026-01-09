use crate::prelude::*;

pub trait EventAttestationProduction {
    type Error;

    async fn note_attestation_production(
        &mut self,
        latest_attestation_eth: common::types::Height,
    ) -> Result<(), Self::Error>;
}

pub trait EventAttestationFinalization {
    type Error;

    async fn note_attestation_finalization(
        &mut self,
        latest_attestation_cc3: common::types::Height,
    ) -> Result<(), Self::Error>;
}

pub trait EventAttestationIntervalChange {
    type Error;

    async fn note_attestation_interval_change(
        &mut self,
        interval_new: std::num::NonZero<common::types::Height>,
        attestation_latest_cc3: Option<common::types::Height>,
    ) -> Result<(), Self::Error>;
}

pub trait EventAttestorsElected {
    type Error;

    async fn note_attestors_elected(
        &mut self,
        attestors: Vec<cc_client::AccountId32>,
    ) -> Result<(), Self::Error>;
}
