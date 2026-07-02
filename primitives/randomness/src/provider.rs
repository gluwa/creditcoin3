use crate::Randomness;
pub trait RandomnessPalletProvider {
    fn randomness_by_epoch_id(epoch_id: u64) -> Randomness;
}

/// No-op provider for tests/mocks that don't exercise randomness-dependent paths.
/// Always yields the zero seed; real runtimes wire the actual randomness pallet instead.
impl RandomnessPalletProvider for () {
    fn randomness_by_epoch_id(_epoch_id: u64) -> Randomness {
        Randomness::default()
    }
}
