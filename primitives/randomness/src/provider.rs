use crate::Randomness;
pub trait RandomnessPalletProvider {
    /// The randomness on record for `epoch_id`, or `None` when there is none.
    ///
    /// `None` covers both an epoch that has not happened yet and one already evicted from the
    /// pallet's bounded retention window — storage cannot tell the two apart. Callers that act on
    /// the seed must handle `None` rather than fall back to the zero seed: `[0; 32]` is a
    /// valid-looking but fully predictable value, so substituting it silently turns "no entropy
    /// available" into "entropy an attacker can precompute".
    fn try_randomness_by_epoch_id(epoch_id: u64) -> Option<Randomness>;

    /// Lossy accessor that substitutes the zero seed for an epoch with no randomness on record.
    ///
    /// Exists for [`crate::api::RandomnessPalletApi`], whose runtime-API signature is infallible.
    /// Prefer [`Self::try_randomness_by_epoch_id`] anywhere the difference between "recorded" and
    /// "absent" can change behaviour.
    fn randomness_by_epoch_id(epoch_id: u64) -> Randomness {
        Self::try_randomness_by_epoch_id(epoch_id).unwrap_or_default()
    }
}

/// Stub provider for runtimes and mocks that never exercise randomness-dependent paths.
///
/// Reports "nothing on record" rather than a zero seed, so a path that unexpectedly grows a
/// dependency on randomness fails loudly here instead of quietly seeding itself with `[0; 32]`.
impl RandomnessPalletProvider for () {
    fn try_randomness_by_epoch_id(_epoch_id: u64) -> Option<Randomness> {
        None
    }
}
