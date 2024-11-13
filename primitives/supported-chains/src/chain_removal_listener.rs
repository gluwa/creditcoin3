use attestor_primitives::ChainKey;

#[impl_trait_for_tuples::impl_for_tuples(10)]
pub trait ChainRemovalListener {
    fn on_supported_chain_removed(chain_key: ChainKey);
}
