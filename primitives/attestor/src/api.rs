use parity_scale_codec::Codec;
use sp_core::H256;

pub type Digest = H256;

sp_api::decl_runtime_apis! {
    pub trait AttestorApi<AccountId>
        where AccountId: Codec
    {
        fn is_attestor(attestor: &AccountId) -> bool;

        fn comittee_set_size() -> u32;

        fn last_digest(chain_id: u8) -> Option<Digest>;
    }
}
