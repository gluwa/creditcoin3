use crate::Randomness;
sp_api::decl_runtime_apis! {
    pub trait RandomnessPalletApi
    {
        fn randomness_by_epoch_id(chain_id: u64) -> Option<Randomness>;
    }
}
