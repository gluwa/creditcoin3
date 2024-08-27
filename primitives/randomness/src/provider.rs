use crate::Randomness;
pub trait RandomnessPalletProvider {
    fn randomness_by_epoch_id(chain_id: u64) -> Randomness;
}
