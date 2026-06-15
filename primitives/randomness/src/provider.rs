use crate::Randomness;
pub trait RandomnessPalletProvider {
    fn randomness_by_epoch_id(epoch_id: u64) -> Randomness;
}
