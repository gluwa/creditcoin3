use crate::Randomness;
sp_api::decl_runtime_apis! {
    pub trait RandomnessPalletApi
    {
        fn randomness_by_epoch_id(epoch_id: u64) -> Randomness;
    }
}
