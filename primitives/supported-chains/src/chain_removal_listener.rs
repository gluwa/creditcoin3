use attestor_primitives::ChainKey;

#[impl_trait_for_tuples::impl_for_tuples(10)]
pub trait ChainRemovalListener {
    fn on_supported_chain_removed(chain_key: ChainKey, remove_checkpoints: bool);
}

/// Reports the worst-case weight a [`ChainRemovalListener`] consumes inside `remove_chain`.
///
/// Kept separate because [`ChainRemovalListener`] is derived for tuples, which cannot generate a
/// method returning a non-unit value. Listeners must bound their work per call and defer the
/// rest, so this is a constant that `remove_chain` can charge up front.
pub trait ChainRemovalListenerWeight {
    /// Worst-case weight consumed by `on_supported_chain_removed`. Defaults to zero.
    fn on_supported_chain_removed_weight() -> sp_weights::Weight {
        sp_weights::Weight::zero()
    }
}

/// Zero-weight listener: the unit type does no work, so it reports no weight.
impl ChainRemovalListenerWeight for () {}
