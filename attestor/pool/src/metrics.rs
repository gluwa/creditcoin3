pub trait MetricsAttestationPool: Send + Sync {
    fn update_attestation_delay_quorum(&self, delay: std::time::Duration);

    fn update_attestation_delay_finalization(&self, delay: std::time::Duration);
}

impl MetricsAttestationPool for Box<dyn MetricsAttestationPool> {
    fn update_attestation_delay_quorum(&self, delay: std::time::Duration) {
        self.as_ref().update_attestation_delay_quorum(delay)
    }

    fn update_attestation_delay_finalization(&self, delay: std::time::Duration) {
        self.as_ref().update_attestation_delay_finalization(delay);
    }
}
