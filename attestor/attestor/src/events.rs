use crate::prelude::*;

pub trait EventAttestationFinalizationAsync {
    type Error;

    async fn note_attestation_finalization_async(
        &mut self,
        info: common::types::AttestationInfo,
    ) -> Result<(), Self::Error>;
}

pub trait EventAttestationFinalization: EventAttestationFinalizationAsync {
    fn note_attestation_finalization(
        &mut self,
        info: common::types::AttestationInfo,
    ) -> Result<(), Self::Error> {
        poll_sync_future(self.note_attestation_finalization_async(info))
    }
}

pub trait EventAttestationIntervalChangeAsync {
    type Error;

    async fn note_attestation_interval_change_async(
        &mut self,
        interval_new: std::num::NonZero<common::types::Height>,
        attestation_latest_cc3: common::types::Height,
    ) -> Result<(), Self::Error>;
}

pub trait EventAttestationIntervalChange: EventAttestationIntervalChangeAsync {
    fn note_attestation_interval_change(
        &mut self,
        interval_new: std::num::NonZero<common::types::Height>,
        attestation_latest_cc3: common::types::Height,
    ) -> Result<(), Self::Error> {
        poll_sync_future(
            self.note_attestation_interval_change_async(interval_new, attestation_latest_cc3),
        )
    }
}

pub trait EventCheckpointIntervalChangeAsync {
    type Error;

    async fn note_checkpoint_interval_change_async(
        &mut self,
        interval_new: std::num::NonZero<common::types::Height>,
        attestation_latest_cc3: common::types::Height,
    ) -> Result<(), Self::Error>;
}

pub trait EventCheckpointIntervalChange: EventCheckpointIntervalChangeAsync {
    fn note_checkpoint_interval_change(
        &mut self,
        interval_new: std::num::NonZero<common::types::Height>,
        attestation_latest_cc3: common::types::Height,
    ) -> Result<(), Self::Error> {
        poll_sync_future(
            self.note_checkpoint_interval_change_async(interval_new, attestation_latest_cc3),
        )
    }
}

pub trait EventAttestorsElectedAsync {
    type Error;

    async fn note_attestors_elected_async(
        &mut self,
        attestors: Vec<cc_client::AccountId32>,
    ) -> Result<(), Self::Error>;
}

pub trait EventAttestorsElected: EventAttestorsElectedAsync {
    fn note_attestors_elected(
        &mut self,
        attestors: Vec<cc_client::AccountId32>,
    ) -> Result<(), Self::Error> {
        poll_sync_future(self.note_attestors_elected_async(attestors))
    }
}

fn poll_sync_future<O>(f: impl std::future::Future<Output = O>) -> O {
    let mut fut = std::pin::pin!(f);
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);

    match fut.as_mut().poll(&mut cx) {
        std::task::Poll::Ready(res) => res,
        std::task::Poll::Pending => unreachable!("Sync use of async event"),
    }
}
